use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use bytes::Bytes;
use futures::StreamExt;
use object_store::aws::{AmazonS3, AmazonS3Builder};
use object_store::path::Path as ObjectPath;
use object_store::{
    Error as OsError, ObjectStore as OsObjectStore, PutMode, PutOptions as OsPutOptions,
};
use zeroize::Zeroizing;

use super::traits::{ObjectStore, PutOptions, StoredObject};
use crate::error::{CoreError, CoreResult};
use crate::hash::ContentId;

#[derive(Clone, Debug)]
pub struct S3Config {
    pub bucket: String,
    pub prefix: Option<String>,
    pub endpoint: Option<String>,
    pub region: Option<String>,
    pub force_path_style: bool,
}

#[derive(Clone)]
pub struct S3Credentials {
    pub access_key_id: Zeroizing<String>,
    pub secret_access_key: Zeroizing<String>,
}

impl std::fmt::Debug for S3Credentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("S3Credentials")
            .field("access_key_id", &self.access_key_id)
            .field("secret_access_key", &"***")
            .finish()
    }
}

/// Resolve S3 access keys from a Kubernetes Secret data map (decoded strings).
///
/// Accepted key pairs (first match wins):
/// - `accessKeyId` / `secretAccessKey`
/// - `AWS_ACCESS_KEY_ID` / `AWS_SECRET_ACCESS_KEY`
/// - `access_key` / `secret_key`
pub fn credentials_from_secret_data(data: &HashMap<String, String>) -> CoreResult<S3Credentials> {
    const PAIRS: &[(&str, &str)] = &[
        ("accessKeyId", "secretAccessKey"),
        ("AWS_ACCESS_KEY_ID", "AWS_SECRET_ACCESS_KEY"),
        ("access_key", "secret_key"),
    ];

    for &(access_key, secret_key) in PAIRS {
        if let (Some(ak), Some(sk)) = (data.get(access_key), data.get(secret_key)) {
            let ak = ak.trim();
            let sk = sk.trim();
            if !ak.is_empty() && !sk.is_empty() {
                return Ok(S3Credentials {
                    access_key_id: Zeroizing::new(ak.to_string()),
                    secret_access_key: Zeroizing::new(sk.to_string()),
                });
            }
        }
    }

    Err(CoreError::InvalidArgument(
        "S3 credentials Secret must contain accessKeyId/secretAccessKey \
         (or AWS_ACCESS_KEY_ID/AWS_SECRET_ACCESS_KEY, or access_key/secret_key)"
            .to_string(),
    ))
}

#[derive(Clone)]
pub struct S3Backend {
    config: S3Config,
    store: Arc<dyn OsObjectStore>,
}

impl std::fmt::Debug for S3Backend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("S3Backend")
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}

impl S3Backend {
    pub fn new(config: S3Config, credentials: S3Credentials) -> CoreResult<Self> {
        let region = config
            .region
            .clone()
            .unwrap_or_else(|| "us-east-1".to_string());

        let mut builder = AmazonS3Builder::new()
            .with_bucket_name(&config.bucket)
            .with_access_key_id(credentials.access_key_id.as_str())
            .with_secret_access_key(credentials.secret_access_key.as_str())
            .with_region(region)
            .with_virtual_hosted_style_request(!config.force_path_style);

        if let Some(endpoint) = &config.endpoint {
            builder = builder.with_endpoint(endpoint);
        }

        let store: AmazonS3 = builder
            .build()
            .map_err(|err| CoreError::S3(format!("failed to build S3 client: {err}")))?;

        Ok(Self {
            config,
            store: Arc::new(store),
        })
    }

    /// Build against any object-store implementation (unit tests / alternate backends).
    pub fn with_store(config: S3Config, store: Arc<dyn OsObjectStore>) -> Self {
        Self { config, store }
    }

    pub fn bucket(&self) -> &str {
        &self.config.bucket
    }

    pub fn object_key(&self, id: &ContentId) -> String {
        let hex = id.to_hex();
        match &self.config.prefix {
            Some(prefix) if !prefix.is_empty() => {
                let trimmed = prefix.trim_matches('/');
                if trimmed.is_empty() {
                    format!("{hex}.blob")
                } else {
                    format!("{trimmed}/{hex}.blob")
                }
            }
            _ => format!("{hex}.blob"),
        }
    }

    fn object_path(&self, id: &ContentId) -> ObjectPath {
        ObjectPath::from(self.object_key(id))
    }

    fn map_err(err: OsError) -> CoreError {
        match &err {
            OsError::NotFound { path, .. } => CoreError::NotFound(path.clone()),
            other => CoreError::S3(other.to_string()),
        }
    }

    /// Best-effort reachability: list one object under the configured prefix.
    pub async fn probe(&self) -> CoreResult<()> {
        let prefix = match &self.config.prefix {
            Some(p) if !p.trim_matches('/').is_empty() => {
                ObjectPath::from(format!("{}/", p.trim_matches('/')))
            }
            _ => ObjectPath::from(""),
        };

        self.store
            .list(Some(&prefix))
            .next()
            .await
            .transpose()
            .map_err(|err| CoreError::S3(format!("list probe failed: {err}")))?;
        Ok(())
    }
}

#[async_trait]
impl ObjectStore for S3Backend {
    async fn put(&self, id: &ContentId, data: Bytes, opts: PutOptions) -> CoreResult<StoredObject> {
        let path = self.object_path(id);
        let size = data.len() as u64;

        if opts.skip_if_exists {
            match self.store.head(&path).await {
                Ok(meta) => {
                    return Ok(StoredObject {
                        id: id.clone(),
                        size: meta.size,
                        deduplicated: true,
                    });
                }
                Err(OsError::NotFound { .. }) => {}
                Err(err) => return Err(Self::map_err(err)),
            }
        }

        let put_opts = OsPutOptions {
            mode: if opts.skip_if_exists {
                PutMode::Create
            } else {
                PutMode::Overwrite
            },
            ..OsPutOptions::default()
        };

        match self.store.put_opts(&path, data.into(), put_opts).await {
            Ok(_) => Ok(StoredObject {
                id: id.clone(),
                size,
                deduplicated: false,
            }),
            Err(OsError::AlreadyExists { .. }) if opts.skip_if_exists => {
                let meta = self.store.head(&path).await.map_err(Self::map_err)?;
                Ok(StoredObject {
                    id: id.clone(),
                    size: meta.size,
                    deduplicated: true,
                })
            }
            Err(err) => Err(Self::map_err(err)),
        }
    }

    async fn get(&self, id: &ContentId) -> CoreResult<Bytes> {
        let path = self.object_path(id);
        let result = self.store.get(&path).await.map_err(Self::map_err)?;
        let bytes = result.bytes().await.map_err(Self::map_err)?;
        Ok(bytes)
    }

    async fn exists(&self, id: &ContentId) -> CoreResult<bool> {
        let path = self.object_path(id);
        match self.store.head(&path).await {
            Ok(_) => Ok(true),
            Err(OsError::NotFound { .. }) => Ok(false),
            Err(err) => Err(Self::map_err(err)),
        }
    }

    async fn delete(&self, id: &ContentId) -> CoreResult<()> {
        let path = self.object_path(id);
        match self.store.delete(&path).await {
            Ok(()) => Ok(()),
            Err(OsError::NotFound { .. }) => Ok(()),
            Err(err) => Err(Self::map_err(err)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hash::hash_bytes;
    use object_store::memory::InMemory;

    fn memory_backend(prefix: Option<&str>) -> S3Backend {
        S3Backend::with_store(
            S3Config {
                bucket: "test".into(),
                prefix: prefix.map(str::to_string),
                endpoint: None,
                region: Some("us-east-1".into()),
                force_path_style: true,
            },
            Arc::new(InMemory::new()),
        )
    }

    #[test]
    fn parses_camel_case_secret_keys() {
        let mut data = HashMap::new();
        data.insert("accessKeyId".into(), "AKIA".into());
        data.insert("secretAccessKey".into(), "secret".into());
        let creds = credentials_from_secret_data(&data).expect("parse");
        assert_eq!(creds.access_key_id.as_str(), "AKIA");
        assert_eq!(creds.secret_access_key.as_str(), "secret");
    }

    #[test]
    fn credentials_debug_redacts_secret() {
        let creds = S3Credentials {
            access_key_id: Zeroizing::new("AKIA".into()),
            secret_access_key: Zeroizing::new("super-secret".into()),
        };
        let dbg = format!("{creds:?}");
        assert!(dbg.contains("AKIA"));
        assert!(dbg.contains("***"));
        assert!(!dbg.contains("super-secret"));
    }

    #[test]
    fn parses_aws_env_style_secret_keys() {
        let mut data = HashMap::new();
        data.insert("AWS_ACCESS_KEY_ID".into(), "id".into());
        data.insert("AWS_SECRET_ACCESS_KEY".into(), "sec".into());
        let creds = credentials_from_secret_data(&data).expect("parse");
        assert_eq!(creds.access_key_id.as_str(), "id");
    }

    #[test]
    fn rejects_missing_secret_keys() {
        let data = HashMap::new();
        let err = credentials_from_secret_data(&data).expect_err("missing");
        assert!(err.to_string().contains("accessKeyId"));
    }

    #[test]
    fn object_key_honours_prefix() {
        let backend = memory_backend(Some("cas/"));
        let id =
            ContentId::from_hex("0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef")
                .expect("hex");
        assert_eq!(
            backend.object_key(&id),
            "cas/0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef.blob"
        );
    }

    #[tokio::test]
    async fn put_get_round_trip() {
        let store = memory_backend(Some("cas"));
        let data = Bytes::from_static(b"s3-cas-bytes");
        let id = hash_bytes(&data);
        store
            .put(&id, data.clone(), PutOptions::default())
            .await
            .expect("put");
        let got = store.get(&id).await.expect("get");
        assert_eq!(got, data);
        assert!(store.exists(&id).await.expect("exists"));
    }

    #[tokio::test]
    async fn skip_if_exists_deduplicates() {
        let store = memory_backend(None);
        let data = Bytes::from_static(b"dup");
        let id = hash_bytes(&data);
        store
            .put(&id, data.clone(), PutOptions::default())
            .await
            .expect("put");
        let second = store
            .put(
                &id,
                data,
                PutOptions {
                    skip_if_exists: true,
                },
            )
            .await
            .expect("dedup put");
        assert!(second.deduplicated);
    }

    #[tokio::test]
    async fn delete_removes_object() {
        let store = memory_backend(None);
        let data = Bytes::from_static(b"bye");
        let id = hash_bytes(&data);
        store
            .put(&id, data, PutOptions::default())
            .await
            .expect("put");
        store.delete(&id).await.expect("delete");
        assert!(!store.exists(&id).await.expect("exists"));
    }

    #[tokio::test]
    async fn get_missing_maps_not_found() {
        let store = memory_backend(None);
        let id = hash_bytes(b"missing");
        let err = store.get(&id).await.expect_err("missing");
        assert!(matches!(err, CoreError::NotFound(_)));
    }

    /// Optional live MinIO smoke: set PROTEUS_S3_ENDPOINT (and optional key/secret/bucket).
    #[tokio::test]
    async fn live_minio_round_trip_when_configured() {
        let Ok(endpoint) = std::env::var("PROTEUS_S3_ENDPOINT") else {
            return;
        };
        let access = std::env::var("PROTEUS_S3_ACCESS_KEY").unwrap_or_else(|_| "minio".into());
        let secret = std::env::var("PROTEUS_S3_SECRET_KEY").unwrap_or_else(|_| "minio123".into());
        let bucket = std::env::var("PROTEUS_S3_BUCKET").unwrap_or_else(|_| "proteus".into());

        let backend = S3Backend::new(
            S3Config {
                bucket,
                prefix: Some("cas-test".into()),
                endpoint: Some(endpoint),
                region: Some("us-east-1".into()),
                force_path_style: true,
            },
            S3Credentials {
                access_key_id: Zeroizing::new(access),
                secret_access_key: Zeroizing::new(secret),
            },
        )
        .expect("client");

        backend.probe().await.expect("probe");
        let data = Bytes::from_static(b"minio-live");
        let id = hash_bytes(&data);
        backend
            .put(&id, data.clone(), PutOptions::default())
            .await
            .expect("put");
        let got = backend.get(&id).await.expect("get");
        assert_eq!(got, data);
        backend.delete(&id).await.expect("delete");
    }
}

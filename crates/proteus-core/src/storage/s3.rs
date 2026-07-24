use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use bytes::Bytes;
use futures::StreamExt;
use object_store::aws::{AmazonS3, AmazonS3Builder};
use object_store::path::Path as ObjectPath;
use object_store::ObjectStore as _;
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

#[derive(Clone, Debug)]
pub struct S3Credentials {
    pub access_key_id: Zeroizing<String>,
    pub secret_access_key: Zeroizing<String>,
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

#[derive(Clone, Debug)]
pub struct S3Backend {
    config: S3Config,
    store: Arc<AmazonS3>,
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

        let store = builder
            .build()
            .map_err(|err| CoreError::S3(format!("failed to build S3 client: {err}")))?;

        Ok(Self {
            config,
            store: Arc::new(store),
        })
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
    async fn put(
        &self,
        _id: &ContentId,
        _data: Bytes,
        _opts: PutOptions,
    ) -> CoreResult<StoredObject> {
        Err(CoreError::S3NotImplemented(format!(
            "put into bucket {}",
            self.config.bucket
        )))
    }

    async fn get(&self, id: &ContentId) -> CoreResult<Bytes> {
        Err(CoreError::S3NotImplemented(format!(
            "get {id} from bucket {}",
            self.config.bucket
        )))
    }

    async fn exists(&self, id: &ContentId) -> CoreResult<bool> {
        Err(CoreError::S3NotImplemented(format!(
            "exists {id} in bucket {}",
            self.config.bucket
        )))
    }

    async fn delete(&self, id: &ContentId) -> CoreResult<()> {
        Err(CoreError::S3NotImplemented(format!(
            "delete {id} from bucket {}",
            self.config.bucket
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_backend(prefix: Option<&str>) -> S3Backend {
        S3Backend::new(
            S3Config {
                bucket: "b".into(),
                prefix: prefix.map(str::to_string),
                endpoint: Some("http://127.0.0.1:9000".into()),
                region: Some("us-east-1".into()),
                force_path_style: true,
            },
            S3Credentials {
                access_key_id: Zeroizing::new("a".into()),
                secret_access_key: Zeroizing::new("s".into()),
            },
        )
        .expect("build")
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
        let backend = test_backend(Some("cas/"));
        let id =
            ContentId::from_hex("0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef")
                .expect("hex");
        assert_eq!(
            backend.object_key(&id),
            "cas/0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef.blob"
        );
    }
}

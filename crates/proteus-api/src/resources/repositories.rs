use k8s_openapi::api::core::v1::Secret;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
use kube::api::{DeleteParams, ListParams, Patch, PatchParams, PostParams};
use kube::{Api, ResourceExt};
use proteus_crd::{
    LocalBackendSpec, ProteusRepository, ProteusRepositorySpec, RepositoryBackend, S3BackendSpec,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::common::{optional_trimmed, phase_label, require_non_empty, resolve_namespace};
use super::secrets::{
    upsert_encryption_secret, upsert_s3_credentials_secret, validate_existing_encryption_secret,
    InlineEncryptionKey, InlineS3Credentials,
};
use crate::error::{ApiError, ApiResult};
use crate::state::{object_namespace, ApiState};

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RepositoryListItem {
    pub name: String,
    pub namespace: String,
    pub phase: Option<String>,
    pub backend: Option<String>,
    pub message: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateRepositoryRequest {
    pub name: String,
    #[serde(default)]
    pub namespace: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub encryption_enabled: Option<bool>,
    /// Existing Secret name; omit (with `encryptionEnabled: true`) to have Proteus generate a key.
    #[serde(default)]
    pub encryption_secret_ref: Option<String>,
    pub backend: CreateRepositoryBackend,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", tag = "type")]
pub enum CreateRepositoryBackend {
    #[serde(rename = "local")]
    Local {
        #[serde(default)]
        path: Option<String>,
    },
    #[serde(rename = "s3")]
    S3 {
        #[serde(default)]
        bucket: Option<String>,
        #[serde(default)]
        prefix: Option<String>,
        #[serde(default)]
        endpoint: Option<String>,
        #[serde(default)]
        region: Option<String>,
        /// Existing Secret name, or desired name when creating from inline keys.
        #[serde(default, rename = "credentialsSecretRef")]
        credentials_secret_ref: Option<String>,
        /// When set with `secretAccessKey`, Proteus creates/updates the Secret.
        #[serde(default, rename = "accessKeyId")]
        access_key_id: Option<String>,
        #[serde(default, rename = "secretAccessKey")]
        secret_access_key: Option<String>,
        #[serde(default)]
        force_path_style: Option<bool>,
    },
}

#[derive(Clone, Debug)]
pub struct PreparedRepository {
    pub namespace: String,
    pub repo: ProteusRepository,
    pub inline_s3_credentials: Option<InlineS3Credentials>,
    pub inline_encryption_key: Option<InlineEncryptionKey>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PatchRepositoryRequest {
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub encryption_enabled: Option<bool>,
    #[serde(default)]
    pub backend: Option<CreateRepositoryBackend>,
}

fn repository_list_item(obj: &ProteusRepository) -> RepositoryListItem {
    let backend = match &obj.spec.backend {
        RepositoryBackend::Local(_) => Some("local".to_string()),
        RepositoryBackend::S3(_) => Some("s3".to_string()),
    };
    RepositoryListItem {
        name: obj.name_any(),
        namespace: object_namespace(obj),
        phase: obj
            .status
            .as_ref()
            .and_then(|s| s.phase.as_ref().and_then(phase_label)),
        backend,
        message: obj.status.as_ref().and_then(|s| s.message.clone()),
    }
}

/// Resolve S3 credentials: either an existing Secret ref, or inline keys to write.
fn resolve_s3_credentials(
    repo_name: &str,
    credentials_secret_ref: Option<&str>,
    access_key_id: Option<&str>,
    secret_access_key: Option<&str>,
) -> ApiResult<(String, Option<InlineS3Credentials>)> {
    let access_key_id = optional_trimmed(access_key_id);
    let secret_access_key = optional_trimmed(secret_access_key);
    let credentials_secret_ref = optional_trimmed(credentials_secret_ref);

    match (access_key_id, secret_access_key, credentials_secret_ref) {
        (Some(access_key_id), Some(secret_access_key), secret_ref) => {
            let secret_name =
                secret_ref.unwrap_or_else(|| format!("{repo_name}-s3-creds"));
            Ok((
                secret_name.clone(),
                Some(InlineS3Credentials {
                    secret_name,
                    access_key_id,
                    secret_access_key,
                }),
            ))
        }
        (None, None, Some(secret_name)) => Ok((secret_name, None)),
        (Some(_), None, _) | (None, Some(_), _) => Err(ApiError::BadRequest(
            "backend.accessKeyId and backend.secretAccessKey must both be provided".to_string(),
        )),
        (None, None, None) => Err(ApiError::BadRequest(
            "provide backend.accessKeyId + backend.secretAccessKey, or backend.credentialsSecretRef"
                .to_string(),
        )),
    }
}

pub fn backend_from_request(
    backend: &CreateRepositoryBackend,
    repo_name: &str,
) -> ApiResult<(RepositoryBackend, Option<InlineS3Credentials>)> {
    match backend {
        CreateRepositoryBackend::Local { path } => {
            let path = require_non_empty("backend.path", path.as_deref())?;
            Ok((RepositoryBackend::Local(LocalBackendSpec { path }), None))
        }
        CreateRepositoryBackend::S3 {
            bucket,
            prefix,
            endpoint,
            region,
            credentials_secret_ref,
            access_key_id,
            secret_access_key,
            force_path_style,
        } => {
            let bucket = require_non_empty("backend.bucket", bucket.as_deref())?;
            let (credentials_secret_ref, inline) = resolve_s3_credentials(
                repo_name,
                credentials_secret_ref.as_deref(),
                access_key_id.as_deref(),
                secret_access_key.as_deref(),
            )?;
            Ok((
                RepositoryBackend::S3(S3BackendSpec {
                    bucket,
                    prefix: optional_trimmed(prefix.as_deref()),
                    endpoint: optional_trimmed(endpoint.as_deref()),
                    region: optional_trimmed(region.as_deref()),
                    credentials_secret_ref,
                    // Default true: MinIO and other S3-compatible endpoints need path-style.
                    force_path_style: force_path_style.unwrap_or(true),
                }),
                inline,
            ))
        }
    }
}

/// When `encryption_enabled`: reuse a provided Secret ref as-is, or generate a fresh key +
/// Secret name (`<repo>-encryption`) for the caller to materialize before/after CR create.
fn resolve_encryption(
    repo_name: &str,
    encryption_enabled: bool,
    encryption_secret_ref: Option<&str>,
) -> (bool, Option<String>, Option<InlineEncryptionKey>) {
    if !encryption_enabled {
        return (false, None, None);
    }
    match optional_trimmed(encryption_secret_ref) {
        Some(secret_ref) => (true, Some(secret_ref), None),
        None => {
            let secret_name = format!("{repo_name}-encryption");
            let key_base64 = proteus_core::EncryptionKey::generate()
                .to_base64()
                .to_string();
            (
                true,
                Some(secret_name.clone()),
                Some(InlineEncryptionKey {
                    secret_name,
                    key_base64,
                }),
            )
        }
    }
}

pub fn build_repository(req: &CreateRepositoryRequest) -> ApiResult<PreparedRepository> {
    let name = require_non_empty("name", Some(req.name.as_str()))?;
    let namespace = resolve_namespace(req.namespace.as_deref())?;
    let (backend, inline_s3_credentials) = backend_from_request(&req.backend, &name)?;
    let description = req
        .description
        .as_ref()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    let (encryption_enabled, encryption_secret_ref, inline_encryption_key) = resolve_encryption(
        &name,
        req.encryption_enabled.unwrap_or(false),
        req.encryption_secret_ref.as_deref(),
    );

    let repo = ProteusRepository {
        metadata: ObjectMeta {
            name: Some(name),
            namespace: Some(namespace.clone()),
            ..ObjectMeta::default()
        },
        spec: ProteusRepositorySpec {
            backend,
            description,
            encryption_enabled,
            encryption_secret_ref,
        },
        status: None,
    };
    Ok(PreparedRepository {
        namespace,
        repo,
        inline_s3_credentials,
        inline_encryption_key,
    })
}

pub async fn list_repositories(state: &ApiState) -> ApiResult<Vec<RepositoryListItem>> {
    let api: Api<ProteusRepository> = Api::all(state.client.clone());
    let list = api.list(&ListParams::default()).await?;
    Ok(list.items.iter().map(repository_list_item).collect())
}

pub async fn get_repository(
    state: &ApiState,
    namespace: &str,
    name: &str,
) -> ApiResult<RepositoryListItem> {
    let api: Api<ProteusRepository> = Api::namespaced(state.client.clone(), namespace);
    let obj = api.get(name).await?;
    Ok(repository_list_item(&obj))
}

pub async fn create_repository(
    state: &ApiState,
    req: CreateRepositoryRequest,
) -> ApiResult<RepositoryListItem> {
    let prepared = build_repository(&req)?;
    let namespace = prepared.namespace.clone();

    // Materialize credentials Secret before the CR so the first reconcile can probe.
    if let Some(creds) = &prepared.inline_s3_credentials {
        upsert_s3_credentials_secret(state, &namespace, creds, None).await?;
    }

    match &prepared.inline_encryption_key {
        Some(key) => upsert_encryption_secret(state, &namespace, key, None).await?,
        None => {
            // A caller-supplied ref (not generated here) must already be valid.
            if let Some(secret_ref) = &prepared.repo.spec.encryption_secret_ref {
                validate_existing_encryption_secret(state, &namespace, secret_ref).await?;
            }
        }
    }

    let api: Api<ProteusRepository> = Api::namespaced(state.client.clone(), &namespace);
    let created = match api.create(&PostParams::default(), &prepared.repo).await {
        Ok(obj) => obj,
        Err(err) => {
            // Best-effort cleanup of Secrets we just created for this failed request.
            let secrets: Api<Secret> = Api::namespaced(state.client.clone(), &namespace);
            if let Some(creds) = &prepared.inline_s3_credentials {
                let _ = secrets
                    .delete(&creds.secret_name, &DeleteParams::default())
                    .await;
            }
            if let Some(key) = &prepared.inline_encryption_key {
                let _ = secrets
                    .delete(&key.secret_name, &DeleteParams::default())
                    .await;
            }
            return Err(err.into());
        }
    };

    // Attach ownerReference so the Secret is GC'd with the repository.
    if let Some(creds) = &prepared.inline_s3_credentials {
        let _ = upsert_s3_credentials_secret(state, &namespace, creds, Some(&created)).await;
    }
    if let Some(key) = &prepared.inline_encryption_key {
        let _ = upsert_encryption_secret(state, &namespace, key, Some(&created)).await;
    }

    let _ = state.refresh_counts().await;
    Ok(repository_list_item(&created))
}

pub async fn patch_repository(
    state: &ApiState,
    namespace: &str,
    name: &str,
    req: PatchRepositoryRequest,
) -> ApiResult<RepositoryListItem> {
    if req.description.is_none() && req.encryption_enabled.is_none() && req.backend.is_none() {
        return Err(ApiError::BadRequest(
            "at least one of description, encryptionEnabled, or backend is required".to_string(),
        ));
    }

    let mut patch = serde_json::Map::new();
    let mut spec = serde_json::Map::new();

    if let Some(description) = &req.description {
        let trimmed = description.trim();
        if trimmed.is_empty() {
            spec.insert("description".to_string(), Value::Null);
        } else {
            spec.insert(
                "description".to_string(),
                Value::String(trimmed.to_string()),
            );
        }
    }
    if let Some(enabled) = req.encryption_enabled {
        spec.insert("encryptionEnabled".to_string(), Value::Bool(enabled));
    }
    let mut inline_creds = None;
    if let Some(backend) = &req.backend {
        let (backend, inline) = backend_from_request(backend, name)?;
        inline_creds = inline;
        spec.insert(
            "backend".to_string(),
            serde_json::to_value(backend)
                .map_err(|err| ApiError::Internal(format!("failed to serialize backend: {err}")))?,
        );
    }

    patch.insert("spec".to_string(), Value::Object(spec));
    let body = Value::Object(patch);

    let api: Api<ProteusRepository> = Api::namespaced(state.client.clone(), namespace);
    let updated = api
        .patch(name, &PatchParams::default(), &Patch::Merge(&body))
        .await?;

    if let Some(creds) = inline_creds {
        upsert_s3_credentials_secret(state, namespace, &creds, Some(&updated)).await?;
    }

    Ok(repository_list_item(&updated))
}

pub async fn delete_repository(state: &ApiState, namespace: &str, name: &str) -> ApiResult<()> {
    let api: Api<ProteusRepository> = Api::namespaced(state.client.clone(), namespace);
    api.delete(name, &DeleteParams::default()).await?;
    let _ = state.refresh_counts().await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use proteus_crd::RepositoryBackend;

    fn s3_backend(
        bucket: Option<&str>,
        credentials_secret_ref: Option<&str>,
        access_key_id: Option<&str>,
        secret_access_key: Option<&str>,
        force_path_style: Option<bool>,
    ) -> CreateRepositoryBackend {
        CreateRepositoryBackend::S3 {
            bucket: bucket.map(str::to_string),
            prefix: None,
            endpoint: None,
            region: None,
            credentials_secret_ref: credentials_secret_ref.map(str::to_string),
            access_key_id: access_key_id.map(str::to_string),
            secret_access_key: secret_access_key.map(str::to_string),
            force_path_style,
        }
    }

    #[test]
    fn rejects_missing_local_path() {
        let req = CreateRepositoryRequest {
            name: "r1".into(),
            namespace: None,
            description: None,
            encryption_enabled: None,
            encryption_secret_ref: None,
            backend: CreateRepositoryBackend::Local { path: None },
        };
        let err = build_repository(&req).expect_err("path required");
        assert!(err.to_string().contains("backend.path"));
    }

    #[test]
    fn rejects_empty_s3_bucket() {
        let req = CreateRepositoryRequest {
            name: "r1".into(),
            namespace: Some("proteus-system".into()),
            description: None,
            encryption_enabled: None,
            encryption_secret_ref: None,
            backend: s3_backend(Some("  "), Some("s3-creds"), None, None, None),
        };
        let err = build_repository(&req).expect_err("bucket required");
        assert!(err.to_string().contains("backend.bucket"));
    }

    #[test]
    fn rejects_missing_s3_credentials() {
        let req = CreateRepositoryRequest {
            name: "r1".into(),
            namespace: None,
            description: None,
            encryption_enabled: None,
            encryption_secret_ref: None,
            backend: s3_backend(Some("backups"), None, None, None, Some(true)),
        };
        let err = build_repository(&req).expect_err("credentials required");
        assert!(err.to_string().contains("accessKeyId"));
    }

    #[test]
    fn rejects_partial_inline_s3_credentials() {
        let req = CreateRepositoryRequest {
            name: "r1".into(),
            namespace: None,
            description: None,
            encryption_enabled: None,
            encryption_secret_ref: None,
            backend: s3_backend(Some("backups"), None, Some("AKIA"), None, None),
        };
        let err = build_repository(&req).expect_err("both keys required");
        assert!(err.to_string().contains("must both be provided"));
    }

    #[test]
    fn builds_local_repo_with_default_namespace() {
        let req = CreateRepositoryRequest {
            name: "local-1".into(),
            namespace: None,
            description: Some("dev".into()),
            encryption_enabled: Some(true),
            encryption_secret_ref: None,
            backend: CreateRepositoryBackend::Local {
                path: Some("/var/lib/proteus/repo".into()),
            },
        };
        let prepared = build_repository(&req).expect("valid");
        assert_eq!(prepared.namespace, "proteus-system");
        assert_eq!(prepared.repo.metadata.name.as_deref(), Some("local-1"));
        assert_eq!(
            prepared.repo.metadata.namespace.as_deref(),
            Some("proteus-system")
        );
        assert!(prepared.repo.spec.encryption_enabled);
        assert!(prepared.inline_s3_credentials.is_none());
        assert!(matches!(
            prepared.repo.spec.backend,
            RepositoryBackend::Local(ref local) if local.path == "/var/lib/proteus/repo"
        ));
    }

    #[test]
    fn builds_s3_repo_from_existing_secret_ref() {
        let req = CreateRepositoryRequest {
            name: "s3-1".into(),
            namespace: Some("demo".into()),
            description: None,
            encryption_enabled: None,
            encryption_secret_ref: None,
            backend: CreateRepositoryBackend::S3 {
                bucket: Some("proteus".into()),
                prefix: Some("cas/".into()),
                endpoint: Some("http://minio.local:9000".into()),
                region: Some("us-east-1".into()),
                credentials_secret_ref: Some("minio-creds".into()),
                access_key_id: None,
                secret_access_key: None,
                force_path_style: Some(true),
            },
        };
        let prepared = build_repository(&req).expect("valid");
        assert_eq!(prepared.namespace, "demo");
        assert!(prepared.inline_s3_credentials.is_none());
        assert!(matches!(
            prepared.repo.spec.backend,
            RepositoryBackend::S3(ref s3)
                if s3.bucket == "proteus"
                    && s3.credentials_secret_ref == "minio-creds"
                    && s3.force_path_style
        ));
    }

    #[test]
    fn builds_s3_repo_from_inline_keys_with_default_secret_name() {
        let req = CreateRepositoryRequest {
            name: "scw-repo".into(),
            namespace: Some("default".into()),
            description: None,
            encryption_enabled: None,
            encryption_secret_ref: None,
            backend: s3_backend(
                Some("my-bucket"),
                None,
                Some("SCWXXXX"),
                Some("secret-value"),
                Some(true),
            ),
        };
        let prepared = build_repository(&req).expect("valid");
        let inline = prepared.inline_s3_credentials.expect("inline creds");
        assert_eq!(inline.secret_name, "scw-repo-s3-creds");
        assert_eq!(inline.access_key_id, "SCWXXXX");
        assert!(matches!(
            prepared.repo.spec.backend,
            RepositoryBackend::S3(ref s3) if s3.credentials_secret_ref == "scw-repo-s3-creds"
        ));
    }

    #[test]
    fn s3_force_path_style_defaults_true_when_omitted() {
        let req = CreateRepositoryRequest {
            name: "s3-default".into(),
            namespace: None,
            description: None,
            encryption_enabled: None,
            encryption_secret_ref: None,
            backend: s3_backend(Some("proteus"), Some("minio-creds"), None, None, None),
        };
        let prepared = build_repository(&req).expect("valid");
        assert!(matches!(
            prepared.repo.spec.backend,
            RepositoryBackend::S3(ref s3) if s3.force_path_style
        ));
    }

    #[test]
    fn resolve_encryption_generates_key_when_enabled_without_ref() {
        let (enabled, secret_ref, inline) = resolve_encryption("repo-1", true, None);
        assert!(enabled);
        assert_eq!(secret_ref.as_deref(), Some("repo-1-encryption"));
        let inline = inline.expect("generated key");
        assert_eq!(inline.secret_name, "repo-1-encryption");
        assert!(!inline.key_base64.is_empty());
    }

    #[test]
    fn resolve_encryption_reuses_provided_ref_without_generating() {
        let (enabled, secret_ref, inline) =
            resolve_encryption("repo-1", true, Some("existing-key"));
        assert!(enabled);
        assert_eq!(secret_ref.as_deref(), Some("existing-key"));
        assert!(inline.is_none());
    }

    #[test]
    fn resolve_encryption_disabled_ignores_ref() {
        let (enabled, secret_ref, inline) = resolve_encryption("repo-1", false, Some("ignored"));
        assert!(!enabled);
        assert!(secret_ref.is_none());
        assert!(inline.is_none());
    }

    #[test]
    fn s3_force_path_style_false_when_explicit() {
        let req = CreateRepositoryRequest {
            name: "s3-aws".into(),
            namespace: None,
            description: None,
            encryption_enabled: None,
            encryption_secret_ref: None,
            backend: s3_backend(Some("proteus"), Some("aws-creds"), None, None, Some(false)),
        };
        let prepared = build_repository(&req).expect("valid");
        assert!(matches!(
            prepared.repo.spec.backend,
            RepositoryBackend::S3(ref s3) if !s3.force_path_style
        ));
    }
}

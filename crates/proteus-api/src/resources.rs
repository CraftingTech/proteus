use k8s_openapi::api::core::v1::Secret;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::{ObjectMeta, OwnerReference};
use kube::api::{DeleteParams, ListParams, Patch, PatchParams, PostParams};
use kube::{Api, ResourceExt};
use proteus_crd::{
    LocalBackendSpec, ProteusBackup, ProteusRepository, ProteusRepositorySpec, ProteusRestore,
    RepositoryBackend, S3BackendSpec,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

use crate::error::{ApiError, ApiResult};
use crate::state::{object_namespace, ApiState};

const DEFAULT_REPO_NAMESPACE: &str = "proteus-system";
const MANAGED_SECRET_LABEL: &str = "proteus.io/managed-credentials";
const MANAGED_SECRET_REPO_LABEL: &str = "proteus.io/repository";

fn phase_label<T: Serialize>(phase: &T) -> Option<String> {
    match serde_json::to_value(phase) {
        Ok(Value::String(label)) => Some(label),
        _ => None,
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RepositoryListItem {
    pub name: String,
    pub namespace: String,
    pub phase: Option<String>,
    pub backend: Option<String>,
    pub message: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupListItem {
    pub name: String,
    pub namespace: String,
    pub repository_ref: String,
    pub target_namespace: String,
    pub schedule: Option<String>,
    pub phase: Option<String>,
    pub message: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RestoreListItem {
    pub name: String,
    pub namespace: String,
    pub backup_ref: String,
    pub target_namespace: String,
    pub phase: Option<String>,
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

/// Inline S3 credentials to materialize as a Kubernetes Secret before/after CR create.
#[derive(Clone, Debug)]
pub struct InlineS3Credentials {
    pub secret_name: String,
    pub access_key_id: String,
    pub secret_access_key: String,
}

#[derive(Clone, Debug)]
pub struct PreparedRepository {
    pub namespace: String,
    pub repo: ProteusRepository,
    pub inline_s3_credentials: Option<InlineS3Credentials>,
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

fn require_non_empty(field: &str, value: Option<&str>) -> ApiResult<String> {
    match value.map(str::trim).filter(|s| !s.is_empty()) {
        Some(v) => Ok(v.to_string()),
        None => Err(ApiError::BadRequest(format!("{field} is required"))),
    }
}

fn resolve_namespace(requested: Option<&str>) -> ApiResult<String> {
    match requested.map(str::trim).filter(|s| !s.is_empty()) {
        Some(ns) => Ok(ns.to_string()),
        None => Ok(DEFAULT_REPO_NAMESPACE.to_string()),
    }
}

fn optional_trimmed(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
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
            Ok((
                RepositoryBackend::Local(LocalBackendSpec { path }),
                None,
            ))
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

pub fn build_repository(req: &CreateRepositoryRequest) -> ApiResult<PreparedRepository> {
    let name = require_non_empty("name", Some(req.name.as_str()))?;
    let namespace = resolve_namespace(req.namespace.as_deref())?;
    let (backend, inline_s3_credentials) = backend_from_request(&req.backend, &name)?;
    let description = req
        .description
        .as_ref()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    let repo = ProteusRepository {
        metadata: ObjectMeta {
            name: Some(name),
            namespace: Some(namespace.clone()),
            ..ObjectMeta::default()
        },
        spec: ProteusRepositorySpec {
            backend,
            description,
            encryption_enabled: req.encryption_enabled.unwrap_or(false),
            encryption_secret_ref: None,
        },
        status: None,
    };
    Ok(PreparedRepository {
        namespace,
        repo,
        inline_s3_credentials,
    })
}

async fn upsert_s3_credentials_secret(
    state: &ApiState,
    namespace: &str,
    creds: &InlineS3Credentials,
    owner: Option<&ProteusRepository>,
) -> ApiResult<()> {
    let mut labels = BTreeMap::new();
    labels.insert(MANAGED_SECRET_LABEL.to_string(), "true".to_string());
    if let Some(repo) = owner {
        labels.insert(
            MANAGED_SECRET_REPO_LABEL.to_string(),
            repo.name_any(),
        );
    }

    let mut string_data = BTreeMap::new();
    string_data.insert("accessKeyId".to_string(), creds.access_key_id.clone());
    string_data.insert(
        "secretAccessKey".to_string(),
        creds.secret_access_key.clone(),
    );

    let owner_references = owner.and_then(|repo| {
        let uid = repo.metadata.uid.as_ref()?;
        Some(vec![OwnerReference {
            api_version: "proteus.io/v1alpha1".to_string(),
            kind: "ProteusRepository".to_string(),
            name: repo.name_any(),
            uid: uid.clone(),
            controller: Some(true),
            block_owner_deletion: Some(true),
        }])
    });

    let secret = Secret {
        metadata: ObjectMeta {
            name: Some(creds.secret_name.clone()),
            namespace: Some(namespace.to_string()),
            labels: Some(labels),
            owner_references,
            ..ObjectMeta::default()
        },
        type_: Some("Opaque".to_string()),
        string_data: Some(string_data),
        ..Secret::default()
    };

    let api: Api<Secret> = Api::namespaced(state.client.clone(), namespace);
    match api.create(&PostParams::default(), &secret).await {
        Ok(_) => Ok(()),
        Err(kube::Error::Api(err)) if err.code == 409 => {
            // Replace credentials on conflict (UI re-submit / recreate).
            let patch = serde_json::json!({
                "metadata": {
                    "labels": {
                        MANAGED_SECRET_LABEL: "true",
                        MANAGED_SECRET_REPO_LABEL: owner.map(|r| r.name_any()).unwrap_or_default(),
                    },
                    "ownerReferences": secret.metadata.owner_references,
                },
                "type": "Opaque",
                "stringData": {
                    "accessKeyId": creds.access_key_id,
                    "secretAccessKey": creds.secret_access_key,
                }
            });
            api.patch(
                &creds.secret_name,
                &PatchParams::apply("proteus-api").force(),
                &Patch::Apply(&patch),
            )
            .await?;
            Ok(())
        }
        Err(err) => Err(err.into()),
    }
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

    let api: Api<ProteusRepository> = Api::namespaced(state.client.clone(), &namespace);
    let created = match api.create(&PostParams::default(), &prepared.repo).await {
        Ok(obj) => obj,
        Err(err) => {
            // Best-effort cleanup of a Secret we just created for this failed request.
            if let Some(creds) = &prepared.inline_s3_credentials {
                let secrets: Api<Secret> = Api::namespaced(state.client.clone(), &namespace);
                let _ = secrets
                    .delete(&creds.secret_name, &DeleteParams::default())
                    .await;
            }
            return Err(err.into());
        }
    };

    // Attach ownerReference so the Secret is GC'd with the repository.
    if let Some(creds) = &prepared.inline_s3_credentials {
        let _ = upsert_s3_credentials_secret(state, &namespace, creds, Some(&created)).await;
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

pub async fn list_backups(state: &ApiState) -> ApiResult<Vec<BackupListItem>> {
    let api: Api<ProteusBackup> = Api::all(state.client.clone());
    let list = api.list(&ListParams::default()).await?;
    Ok(list
        .items
        .into_iter()
        .map(|obj| BackupListItem {
            name: obj.name_any(),
            namespace: object_namespace(&obj),
            repository_ref: obj.spec.repository_ref.clone(),
            target_namespace: obj.spec.target_namespace.clone(),
            schedule: obj.spec.schedule.clone(),
            phase: obj
                .status
                .as_ref()
                .and_then(|s| s.phase.as_ref().and_then(phase_label)),
            message: obj.status.as_ref().and_then(|s| s.message.clone()),
        })
        .collect())
}

pub async fn list_restores(state: &ApiState) -> ApiResult<Vec<RestoreListItem>> {
    let api: Api<ProteusRestore> = Api::all(state.client.clone());
    let list = api.list(&ListParams::default()).await?;
    Ok(list
        .items
        .into_iter()
        .map(|obj| RestoreListItem {
            name: obj.name_any(),
            namespace: object_namespace(&obj),
            backup_ref: obj.spec.backup_ref.clone(),
            target_namespace: obj.spec.target_namespace.clone(),
            phase: obj
                .status
                .as_ref()
                .and_then(|s| s.phase.as_ref().and_then(phase_label)),
            message: obj.status.as_ref().and_then(|s| s.message.clone()),
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

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
            backend: s3_backend(
                Some("backups"),
                None,
                None,
                None,
                Some(true),
            ),
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
            backend: s3_backend(Some("proteus"), Some("minio-creds"), None, None, None),
        };
        let prepared = build_repository(&req).expect("valid");
        assert!(matches!(
            prepared.repo.spec.backend,
            RepositoryBackend::S3(ref s3) if s3.force_path_style
        ));
    }

    #[test]
    fn s3_force_path_style_false_when_explicit() {
        let req = CreateRepositoryRequest {
            name: "s3-aws".into(),
            namespace: None,
            description: None,
            encryption_enabled: None,
            backend: s3_backend(
                Some("proteus"),
                Some("aws-creds"),
                None,
                None,
                Some(false),
            ),
        };
        let prepared = build_repository(&req).expect("valid");
        assert!(matches!(
            prepared.repo.spec.backend,
            RepositoryBackend::S3(ref s3) if !s3.force_path_style
        ));
    }
}

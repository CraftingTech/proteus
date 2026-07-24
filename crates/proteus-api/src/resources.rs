use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
use kube::api::{DeleteParams, ListParams, Patch, PatchParams, PostParams};
use kube::{Api, ResourceExt};
use proteus_crd::{
    LocalBackendSpec, ProteusBackup, ProteusRepository, ProteusRepositorySpec, ProteusRestore,
    RepositoryBackend, S3BackendSpec,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::{ApiError, ApiResult};
use crate::state::{object_namespace, ApiState};

const DEFAULT_REPO_NAMESPACE: &str = "proteus-system";

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
        #[serde(default, rename = "credentialsSecretRef")]
        credentials_secret_ref: Option<String>,
        #[serde(default)]
        force_path_style: Option<bool>,
    },
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

pub fn backend_from_request(backend: &CreateRepositoryBackend) -> ApiResult<RepositoryBackend> {
    match backend {
        CreateRepositoryBackend::Local { path } => {
            let path = require_non_empty("backend.path", path.as_deref())?;
            Ok(RepositoryBackend::Local(LocalBackendSpec { path }))
        }
        CreateRepositoryBackend::S3 {
            bucket,
            prefix,
            endpoint,
            region,
            credentials_secret_ref,
            force_path_style,
        } => {
            let bucket = require_non_empty("backend.bucket", bucket.as_deref())?;
            let credentials_secret_ref = require_non_empty(
                "backend.credentialsSecretRef",
                credentials_secret_ref.as_deref(),
            )?;
            Ok(RepositoryBackend::S3(S3BackendSpec {
                bucket,
                prefix: prefix
                    .as_ref()
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty()),
                endpoint: endpoint
                    .as_ref()
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty()),
                region: region
                    .as_ref()
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty()),
                credentials_secret_ref,
                // Default true: MinIO and other S3-compatible endpoints need path-style.
                force_path_style: force_path_style.unwrap_or(true),
            }))
        }
    }
}

pub fn build_repository(req: &CreateRepositoryRequest) -> ApiResult<(String, ProteusRepository)> {
    let name = require_non_empty("name", Some(req.name.as_str()))?;
    let namespace = resolve_namespace(req.namespace.as_deref())?;
    let backend = backend_from_request(&req.backend)?;
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
    Ok((namespace, repo))
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
    let (namespace, repo) = build_repository(&req)?;
    let api: Api<ProteusRepository> = Api::namespaced(state.client.clone(), &namespace);
    let created = api.create(&PostParams::default(), &repo).await?;
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
    if let Some(backend) = &req.backend {
        let backend = backend_from_request(backend)?;
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
            backend: CreateRepositoryBackend::S3 {
                bucket: Some("  ".into()),
                prefix: None,
                endpoint: None,
                region: None,
                credentials_secret_ref: Some("s3-creds".into()),
                force_path_style: None,
            },
        };
        let err = build_repository(&req).expect_err("bucket required");
        assert!(err.to_string().contains("backend.bucket"));
    }

    #[test]
    fn rejects_missing_s3_credentials_secret_ref() {
        let req = CreateRepositoryRequest {
            name: "r1".into(),
            namespace: None,
            description: None,
            encryption_enabled: None,
            backend: CreateRepositoryBackend::S3 {
                bucket: Some("backups".into()),
                prefix: None,
                endpoint: Some("http://minio:9000".into()),
                region: Some("us-east-1".into()),
                credentials_secret_ref: None,
                force_path_style: Some(true),
            },
        };
        let err = build_repository(&req).expect_err("secret required");
        assert!(err.to_string().contains("backend.credentialsSecretRef"));
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
        let (ns, repo) = build_repository(&req).expect("valid");
        assert_eq!(ns, "proteus-system");
        assert_eq!(repo.metadata.name.as_deref(), Some("local-1"));
        assert_eq!(repo.metadata.namespace.as_deref(), Some("proteus-system"));
        assert!(repo.spec.encryption_enabled);
        assert!(matches!(
            repo.spec.backend,
            RepositoryBackend::Local(ref local) if local.path == "/var/lib/proteus/repo"
        ));
    }

    #[test]
    fn builds_s3_repo() {
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
                force_path_style: Some(true),
            },
        };
        let (ns, repo) = build_repository(&req).expect("valid");
        assert_eq!(ns, "demo");
        assert!(matches!(
            repo.spec.backend,
            RepositoryBackend::S3(ref s3)
                if s3.bucket == "proteus"
                    && s3.credentials_secret_ref == "minio-creds"
                    && s3.force_path_style
        ));
    }

    #[test]
    fn s3_force_path_style_defaults_true_when_omitted() {
        let req = CreateRepositoryRequest {
            name: "s3-default".into(),
            namespace: None,
            description: None,
            encryption_enabled: None,
            backend: CreateRepositoryBackend::S3 {
                bucket: Some("proteus".into()),
                prefix: None,
                endpoint: Some("http://minio:9000".into()),
                region: None,
                credentials_secret_ref: Some("minio-creds".into()),
                force_path_style: None,
            },
        };
        let (_, repo) = build_repository(&req).expect("valid");
        assert!(matches!(
            repo.spec.backend,
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
            backend: CreateRepositoryBackend::S3 {
                bucket: Some("proteus".into()),
                prefix: None,
                endpoint: None,
                region: Some("eu-west-1".into()),
                credentials_secret_ref: Some("aws-creds".into()),
                force_path_style: Some(false),
            },
        };
        let (_, repo) = build_repository(&req).expect("valid");
        assert!(matches!(
            repo.spec.backend,
            RepositoryBackend::S3(ref s3) if !s3.force_path_style
        ));
    }
}

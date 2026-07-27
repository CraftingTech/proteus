use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
use kube::api::{DeleteParams, ListParams, PostParams};
use kube::{Api, ResourceExt};
use proteus_crd::{ProteusBackup, ProteusBackupSpec, RetentionPolicy};
use serde::{Deserialize, Serialize};
use tracing::warn;

use super::common::{optional_trimmed, phase_label, require_non_empty};
use super::repo_store::gc_repository_after_backup_delete;
use crate::error::{ApiError, ApiResult};
use crate::state::{object_namespace, ApiState};

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupListItem {
    pub name: String,
    pub namespace: String,
    pub repository_ref: String,
    pub target_namespace: String,
    pub pvc_names: Vec<String>,
    pub schedule: Option<String>,
    pub phase: Option<String>,
    pub message: Option<String>,
    pub last_snapshot_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub progress_percent: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_seconds: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub throughput_bytes_per_sec: Option<u64>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateBackupRequest {
    pub name: String,
    /// Namespace for the `ProteusBackup` CR itself; defaults to `targetNamespace`.
    #[serde(default)]
    pub namespace: Option<String>,
    pub repository_ref: String,
    #[serde(default)]
    pub repository_namespace: Option<String>,
    pub target_namespace: String,
    #[serde(default)]
    pub pvc_names: Vec<String>,
}

fn backup_list_item(obj: &ProteusBackup) -> BackupListItem {
    BackupListItem {
        name: obj.name_any(),
        namespace: object_namespace(obj),
        repository_ref: obj.spec.repository_ref.clone(),
        target_namespace: obj.spec.target_namespace.clone(),
        pvc_names: obj.spec.pvc_names.clone(),
        schedule: obj.spec.schedule.clone(),
        phase: obj
            .status
            .as_ref()
            .and_then(|s| s.phase.as_ref().and_then(phase_label)),
        message: obj.status.as_ref().and_then(|s| s.message.clone()),
        last_snapshot_id: obj.status.as_ref().and_then(|s| s.last_snapshot_id.clone()),
        progress_percent: obj.status.as_ref().and_then(|s| s.progress_percent),
        duration_seconds: obj.status.as_ref().and_then(|s| s.duration_seconds),
        throughput_bytes_per_sec: obj.status.as_ref().and_then(|s| s.throughput_bytes_per_sec),
    }
}

pub async fn list_backups(state: &ApiState) -> ApiResult<Vec<BackupListItem>> {
    let api: Api<ProteusBackup> = Api::all(state.client.clone());
    let list = api.list(&ListParams::default()).await?;
    Ok(list.items.iter().map(backup_list_item).collect())
}

pub fn build_backup(req: &CreateBackupRequest) -> ApiResult<(String, ProteusBackup)> {
    let name = require_non_empty("name", Some(req.name.as_str()))?;
    let target_namespace =
        require_non_empty("targetNamespace", Some(req.target_namespace.as_str()))?;
    let repository_ref = require_non_empty("repositoryRef", Some(req.repository_ref.as_str()))?;
    let namespace =
        optional_trimmed(req.namespace.as_deref()).unwrap_or_else(|| target_namespace.clone());

    let pvc_names: Vec<String> = req
        .pvc_names
        .iter()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    if pvc_names.is_empty() {
        return Err(ApiError::BadRequest(
            "pvcNames must contain at least one PVC name".to_string(),
        ));
    }

    let backup = ProteusBackup {
        metadata: ObjectMeta {
            name: Some(name),
            namespace: Some(namespace.clone()),
            ..ObjectMeta::default()
        },
        spec: ProteusBackupSpec {
            repository_ref,
            repository_namespace: optional_trimmed(req.repository_namespace.as_deref()),
            target_namespace,
            pvc_names,
            label_selector: None,
            schedule: None,
            retention: RetentionPolicy::default(),
            include_volumes: true,
            include_cluster_resources: false,
        },
        status: None,
    };
    Ok((namespace, backup))
}

pub async fn create_backup(
    state: &ApiState,
    req: CreateBackupRequest,
) -> ApiResult<BackupListItem> {
    let (namespace, backup) = build_backup(&req)?;
    let api: Api<ProteusBackup> = Api::namespaced(state.client.clone(), &namespace);
    let created = api.create(&PostParams::default(), &backup).await?;
    let _ = state.refresh_counts().await;
    Ok(backup_list_item(&created))
}

pub async fn delete_backup(state: &ApiState, namespace: &str, name: &str) -> ApiResult<()> {
    let api: Api<ProteusBackup> = Api::namespaced(state.client.clone(), namespace);
    let backup = api.get(name).await?;

    let repo_ns = backup
        .spec
        .repository_namespace
        .as_deref()
        .unwrap_or(namespace)
        .to_string();
    let repo_ref = backup.spec.repository_ref.clone();

    // GC first: drop this backup's snapshot + orphans; keep other backups' objects.
    match gc_repository_after_backup_delete(state, namespace, name, &repo_ns, &repo_ref).await {
        Ok(removed) => {
            if removed > 0 {
                tracing::info!(
                    backup = %name,
                    repository = %repo_ref,
                    removed,
                    "purged unreferenced objects from repository"
                );
            }
        }
        Err(err) => {
            warn!(
                backup = %name,
                error = %err,
                "failed to purge repository objects; leaving CR in place"
            );
            return Err(ApiError::Internal(format!(
                "could not remove backup data from repository: {err}"
            )));
        }
    }

    api.delete(name, &DeleteParams::default()).await?;
    let _ = state.refresh_counts().await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_backup_rejects_empty_pvc_names() {
        let req = CreateBackupRequest {
            name: "b1".into(),
            namespace: None,
            repository_ref: "repo-1".into(),
            repository_namespace: None,
            target_namespace: "workloads".into(),
            pvc_names: vec![],
        };
        let err = build_backup(&req).expect_err("pvcNames required");
        assert!(err.to_string().contains("pvcNames"));
    }

    #[test]
    fn build_backup_rejects_blank_pvc_names() {
        let req = CreateBackupRequest {
            name: "b1".into(),
            namespace: None,
            repository_ref: "repo-1".into(),
            repository_namespace: None,
            target_namespace: "workloads".into(),
            pvc_names: vec!["  ".into()],
        };
        let err = build_backup(&req).expect_err("blank pvc name filtered out");
        assert!(err.to_string().contains("pvcNames"));
    }

    #[test]
    fn build_backup_defaults_namespace_to_target_namespace() {
        let req = CreateBackupRequest {
            name: "b1".into(),
            namespace: None,
            repository_ref: "repo-1".into(),
            repository_namespace: None,
            target_namespace: "workloads".into(),
            pvc_names: vec!["data-pvc".into()],
        };
        let (namespace, backup) = build_backup(&req).expect("valid");
        assert_eq!(namespace, "workloads");
        assert_eq!(backup.metadata.namespace.as_deref(), Some("workloads"));
        assert_eq!(backup.spec.pvc_names, vec!["data-pvc".to_string()]);
    }

    #[test]
    fn build_backup_honours_explicit_namespace() {
        let req = CreateBackupRequest {
            name: "b1".into(),
            namespace: Some("proteus-system".into()),
            repository_ref: "repo-1".into(),
            repository_namespace: Some("proteus-system".into()),
            target_namespace: "workloads".into(),
            pvc_names: vec!["data-pvc".into(), "logs-pvc".into()],
        };
        let (namespace, backup) = build_backup(&req).expect("valid");
        assert_eq!(namespace, "proteus-system");
        assert_eq!(backup.spec.target_namespace, "workloads");
        assert_eq!(backup.spec.pvc_names.len(), 2);
    }
}

use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
use kube::api::{DeleteParams, ListParams, PostParams};
use kube::{Api, ResourceExt};
use proteus_crd::{ProteusRestore, ProteusRestoreSpec};
use serde::{Deserialize, Serialize};

use super::common::{optional_trimmed, phase_label, require_non_empty};
use crate::error::ApiResult;
use crate::state::{object_namespace, ApiState};

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RestoreListItem {
    pub name: String,
    pub namespace: String,
    pub backup_ref: String,
    pub target_namespace: String,
    pub phase: Option<String>,
    pub message: Option<String>,
    pub restored_snapshot_id: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateRestoreRequest {
    pub name: String,
    /// Namespace for the `ProteusRestore` CR itself; defaults to `targetNamespace`.
    #[serde(default)]
    pub namespace: Option<String>,
    pub backup_ref: String,
    /// Namespace of the source `ProteusBackup`; defaults to the restore's own namespace.
    #[serde(default)]
    pub backup_namespace: Option<String>,
    /// Explicit snapshot id; omit to let the controller resolve the backup's latest snapshot.
    #[serde(default)]
    pub snapshot_id: Option<String>,
    pub target_namespace: String,
    #[serde(default)]
    pub overwrite: bool,
}

fn restore_list_item(obj: &ProteusRestore) -> RestoreListItem {
    RestoreListItem {
        name: obj.name_any(),
        namespace: object_namespace(obj),
        backup_ref: obj.spec.backup_ref.clone(),
        target_namespace: obj.spec.target_namespace.clone(),
        phase: obj
            .status
            .as_ref()
            .and_then(|s| s.phase.as_ref().and_then(phase_label)),
        message: obj.status.as_ref().and_then(|s| s.message.clone()),
        restored_snapshot_id: obj
            .status
            .as_ref()
            .and_then(|s| s.restored_snapshot_id.clone()),
    }
}

pub async fn list_restores(state: &ApiState) -> ApiResult<Vec<RestoreListItem>> {
    let api: Api<ProteusRestore> = Api::all(state.client.clone());
    let list = api.list(&ListParams::default()).await?;
    Ok(list.items.iter().map(restore_list_item).collect())
}

pub fn build_restore(req: &CreateRestoreRequest) -> ApiResult<(String, ProteusRestore)> {
    let name = require_non_empty("name", Some(req.name.as_str()))?;
    let target_namespace =
        require_non_empty("targetNamespace", Some(req.target_namespace.as_str()))?;
    let backup_ref = require_non_empty("backupRef", Some(req.backup_ref.as_str()))?;
    let namespace =
        optional_trimmed(req.namespace.as_deref()).unwrap_or_else(|| target_namespace.clone());

    let restore = ProteusRestore {
        metadata: ObjectMeta {
            name: Some(name),
            namespace: Some(namespace.clone()),
            ..ObjectMeta::default()
        },
        spec: ProteusRestoreSpec {
            backup_ref,
            backup_namespace: optional_trimmed(req.backup_namespace.as_deref()),
            snapshot_id: optional_trimmed(req.snapshot_id.as_deref()),
            target_namespace,
            overwrite: req.overwrite,
            include_resources: None,
        },
        status: None,
    };
    Ok((namespace, restore))
}

pub async fn create_restore(
    state: &ApiState,
    req: CreateRestoreRequest,
) -> ApiResult<RestoreListItem> {
    let (namespace, restore) = build_restore(&req)?;
    let api: Api<ProteusRestore> = Api::namespaced(state.client.clone(), &namespace);
    let created = api.create(&PostParams::default(), &restore).await?;
    let _ = state.refresh_counts().await;
    Ok(restore_list_item(&created))
}

pub async fn delete_restore(state: &ApiState, namespace: &str, name: &str) -> ApiResult<()> {
    let api: Api<ProteusRestore> = Api::namespaced(state.client.clone(), namespace);
    api.delete(name, &DeleteParams::default()).await?;
    let _ = state.refresh_counts().await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_restore_rejects_empty_backup_ref() {
        let req = CreateRestoreRequest {
            name: "r1".into(),
            namespace: None,
            backup_ref: "  ".into(),
            backup_namespace: None,
            snapshot_id: None,
            target_namespace: "workloads".into(),
            overwrite: false,
        };
        let err = build_restore(&req).expect_err("backupRef required");
        assert!(err.to_string().contains("backupRef"));
    }

    #[test]
    fn build_restore_rejects_empty_target_namespace() {
        let req = CreateRestoreRequest {
            name: "r1".into(),
            namespace: None,
            backup_ref: "backup-1".into(),
            backup_namespace: None,
            snapshot_id: None,
            target_namespace: "".into(),
            overwrite: false,
        };
        let err = build_restore(&req).expect_err("targetNamespace required");
        assert!(err.to_string().contains("targetNamespace"));
    }

    #[test]
    fn build_restore_defaults_namespace_to_target_namespace() {
        let req = CreateRestoreRequest {
            name: "r1".into(),
            namespace: None,
            backup_ref: "backup-1".into(),
            backup_namespace: None,
            snapshot_id: None,
            target_namespace: "workloads".into(),
            overwrite: true,
        };
        let (namespace, restore) = build_restore(&req).expect("valid");
        assert_eq!(namespace, "workloads");
        assert_eq!(restore.metadata.namespace.as_deref(), Some("workloads"));
        assert!(restore.spec.overwrite);
        assert!(restore.spec.snapshot_id.is_none());
    }

    #[test]
    fn build_restore_honours_cross_namespace_backup_ref() {
        let req = CreateRestoreRequest {
            name: "r1".into(),
            namespace: Some("proteus-system".into()),
            backup_ref: "backup-1".into(),
            backup_namespace: Some("proteus-system".into()),
            snapshot_id: Some("deadbeef".into()),
            target_namespace: "workloads".into(),
            overwrite: false,
        };
        let (namespace, restore) = build_restore(&req).expect("valid");
        assert_eq!(namespace, "proteus-system");
        assert_eq!(
            restore.spec.backup_namespace.as_deref(),
            Some("proteus-system")
        );
        assert_eq!(restore.spec.snapshot_id.as_deref(), Some("deadbeef"));
        assert_eq!(restore.spec.target_namespace, "workloads");
    }
}

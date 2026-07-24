use std::sync::Arc;
use std::time::Duration;

use kube::api::{Patch, PatchParams};
use kube::runtime::controller::Action;
use kube::{Api, ResourceExt};
use proteus_crd::{BackupPhase, ProteusBackup, ProteusBackupStatus};
use tracing::info;

use super::ReconcileCtx;
use crate::error::{ControllerError, ControllerResult};

pub async fn reconcile_backup(
    obj: Arc<ProteusBackup>,
    ctx: Arc<ReconcileCtx>,
) -> ControllerResult<Action> {
    let ns = match obj.namespace() {
        Some(ns) => ns,
        None => "default".to_string(),
    };
    let name = obj.name_any();
    info!(%ns, %name, "reconciling ProteusBackup");

    validate_spec(&obj)?;

    let api: Api<ProteusBackup> = Api::namespaced(ctx.client.clone(), &ns);
    let status = ProteusBackupStatus {
        phase: Some(BackupPhase::Pending),
        message: Some("backup accepted; scheduler not yet implemented".to_string()),
        last_snapshot_id: obj.status.as_ref().and_then(|s| s.last_snapshot_id.clone()),
        last_success_at: obj.status.as_ref().and_then(|s| s.last_success_at.clone()),
        last_failure_at: obj.status.as_ref().and_then(|s| s.last_failure_at.clone()),
        last_bytes: obj.status.as_ref().and_then(|s| s.last_bytes),
        retained_snapshots: obj.status.as_ref().and_then(|s| s.retained_snapshots),
    };

    let patch = serde_json::json!({ "status": status });
    api.patch_status(&name, &PatchParams::default(), &Patch::Merge(&patch))
        .await?;

    if let Err(err) = ctx.api_state.refresh_counts().await {
        tracing::warn!(error = %err, "failed to refresh cluster snapshot counts");
        ctx.api_state.mark_reconciled();
    }
    Ok(Action::requeue(Duration::from_secs(60)))
}

fn validate_spec(obj: &ProteusBackup) -> ControllerResult<()> {
    if obj.spec.repository_ref.is_empty() {
        return Err(ControllerError::InvalidSpec(
            "repositoryRef must not be empty".to_string(),
        ));
    }
    if obj.spec.target_namespace.is_empty() {
        return Err(ControllerError::InvalidSpec(
            "targetNamespace must not be empty".to_string(),
        ));
    }
    if obj.spec.retention.keep_last == 0 {
        return Err(ControllerError::InvalidSpec(
            "retention.keepLast must be >= 1".to_string(),
        ));
    }
    Ok(())
}

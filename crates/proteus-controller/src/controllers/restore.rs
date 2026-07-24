use std::sync::Arc;
use std::time::Duration;

use kube::api::{Patch, PatchParams};
use kube::runtime::controller::Action;
use kube::{Api, ResourceExt};
use proteus_crd::{ProteusRestore, ProteusRestoreStatus, RestorePhase};
use tracing::info;

use super::ReconcileCtx;
use crate::error::{ControllerError, ControllerResult};

pub async fn reconcile_restore(
    obj: Arc<ProteusRestore>,
    ctx: Arc<ReconcileCtx>,
) -> ControllerResult<Action> {
    let ns = match obj.namespace() {
        Some(ns) => ns,
        None => "default".to_string(),
    };
    let name = obj.name_any();
    info!(%ns, %name, "reconciling ProteusRestore");

    validate_spec(&obj)?;

    let api: Api<ProteusRestore> = Api::namespaced(ctx.client.clone(), &ns);
    let status = ProteusRestoreStatus {
        phase: Some(RestorePhase::Pending),
        message: Some("restore accepted; restore engine not yet implemented".to_string()),
        restored_snapshot_id: obj
            .status
            .as_ref()
            .and_then(|s| s.restored_snapshot_id.clone()),
        progress_percent: obj.status.as_ref().and_then(|s| s.progress_percent).or(Some(0)),
        completed_at: obj.status.as_ref().and_then(|s| s.completed_at.clone()),
    };

    let changed = match obj.status.as_ref() {
        None => true,
        Some(cur) => cur.phase != status.phase || cur.message != status.message,
    };
    if changed {
        let patch = serde_json::json!({ "status": status });
        api.patch_status(&name, &PatchParams::default(), &Patch::Merge(&patch))
            .await?;
    }
    if let Err(err) = ctx.api_state.refresh_counts().await {
        tracing::warn!(error = %err, "failed to refresh cluster snapshot counts");
        ctx.api_state.mark_reconciled();
    }
    Ok(Action::requeue(Duration::from_secs(60)))
}

fn validate_spec(obj: &ProteusRestore) -> ControllerResult<()> {
    if obj.spec.backup_ref.is_empty() {
        return Err(ControllerError::InvalidSpec(
            "backupRef must not be empty".to_string(),
        ));
    }
    if obj.spec.target_namespace.is_empty() {
        return Err(ControllerError::InvalidSpec(
            "targetNamespace must not be empty".to_string(),
        ));
    }
    Ok(())
}

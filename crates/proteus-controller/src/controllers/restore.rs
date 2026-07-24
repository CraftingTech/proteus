use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use kube::api::{Patch, PatchParams};
use kube::runtime::controller::Action;
use kube::{Api, ResourceExt};
use proteus_crd::{ProteusRestore, ProteusRestoreStatus, RestorePhase};
use tracing::{info, warn};

use super::ReconcileCtx;
use crate::error::ControllerResult;

pub async fn reconcile_restore(
    obj: Arc<ProteusRestore>,
    ctx: Arc<ReconcileCtx>,
) -> ControllerResult<Action> {
    let ns = obj.namespace().unwrap_or_else(|| "default".to_string());
    let name = obj.name_any();
    info!(%ns, %name, "reconciling ProteusRestore");

    let api: Api<ProteusRestore> = Api::namespaced(ctx.client.clone(), &ns);

    // Terminal phases never re-run the restore pipeline (avoids flood + pod churn, and repeat
    // overwrites of the target PVC). A retry requires a new ProteusRestore object.
    if matches!(
        current_phase(&obj),
        Some(RestorePhase::Succeeded | RestorePhase::Failed)
    ) {
        refresh_counts(&ctx).await;
        return Ok(Action::requeue(Duration::from_secs(3600)));
    }

    if let Err(message) = validate_spec(&obj) {
        let status = terminal_status(&obj, Err(message));
        if status_changed(obj.status.as_ref(), &status) {
            patch_status(&api, &name, &status).await?;
        }
        refresh_counts(&ctx).await;
        return Ok(Action::requeue(Duration::from_secs(3600)));
    }

    let running = running_status(&obj);
    if status_changed(obj.status.as_ref(), &running) {
        patch_status(&api, &name, &running).await?;
    }

    let outcome = crate::restore::run_restore(&obj, &ctx, &ns).await;
    let status = terminal_status(&obj, outcome);
    if status_changed(obj.status.as_ref(), &status) {
        // After the Running patch, the in-memory obj may be stale; compare against intended
        // terminal fields only — always write the first terminal transition.
        patch_status(&api, &name, &status).await?;
    }

    refresh_counts(&ctx).await;

    let requeue = match status.phase {
        Some(RestorePhase::Succeeded | RestorePhase::Failed) => Duration::from_secs(3600),
        _ => Duration::from_secs(60),
    };
    Ok(Action::requeue(requeue))
}

async fn refresh_counts(ctx: &ReconcileCtx) {
    if let Err(err) = ctx.api_state.refresh_counts().await {
        warn!(error = %err, "failed to refresh cluster snapshot counts");
    } else {
        ctx.api_state.mark_reconciled();
    }
}

fn current_phase(obj: &ProteusRestore) -> Option<RestorePhase> {
    obj.status.as_ref().and_then(|s| s.phase.clone())
}

fn running_status(obj: &ProteusRestore) -> ProteusRestoreStatus {
    ProteusRestoreStatus {
        phase: Some(RestorePhase::Running),
        message: Some("restore running: resolving snapshot and writing target PVCs".to_string()),
        restored_snapshot_id: obj
            .status
            .as_ref()
            .and_then(|s| s.restored_snapshot_id.clone()),
        progress_percent: Some(0),
        completed_at: None,
    }
}

fn terminal_status(obj: &ProteusRestore, outcome: Result<String, String>) -> ProteusRestoreStatus {
    match outcome {
        Ok(snapshot_id) => ProteusRestoreStatus {
            phase: Some(RestorePhase::Succeeded),
            message: Some(format!("restore succeeded from snapshot {snapshot_id}")),
            restored_snapshot_id: Some(snapshot_id),
            progress_percent: Some(100),
            completed_at: Some(Utc::now().to_rfc3339()),
        },
        Err(message) => ProteusRestoreStatus {
            phase: Some(RestorePhase::Failed),
            message: Some(message),
            restored_snapshot_id: obj
                .status
                .as_ref()
                .and_then(|s| s.restored_snapshot_id.clone()),
            progress_percent: obj.status.as_ref().and_then(|s| s.progress_percent),
            completed_at: Some(Utc::now().to_rfc3339()),
        },
    }
}

/// Patch only when phase / stable message change — timestamps alone must not re-trigger.
fn status_changed(current: Option<&ProteusRestoreStatus>, next: &ProteusRestoreStatus) -> bool {
    match current {
        None => true,
        Some(cur) => {
            cur.phase != next.phase
                || message_fingerprint(cur.message.as_deref())
                    != message_fingerprint(next.message.as_deref())
                || cur.restored_snapshot_id != next.restored_snapshot_id
        }
    }
}

fn message_fingerprint(msg: Option<&str>) -> String {
    let msg = msg.unwrap_or("");
    let mut out = String::with_capacity(msg.len());
    let mut in_digit = false;
    for c in msg.chars() {
        if c.is_ascii_digit() {
            if !in_digit {
                out.push('0');
                in_digit = true;
            }
        } else {
            in_digit = false;
            out.push(c);
        }
    }
    out
}

async fn patch_status(
    api: &Api<ProteusRestore>,
    name: &str,
    status: &ProteusRestoreStatus,
) -> ControllerResult<()> {
    let patch = serde_json::json!({ "status": status });
    api.patch_status(name, &PatchParams::default(), &Patch::Merge(&patch))
        .await?;
    Ok(())
}

fn validate_spec(obj: &ProteusRestore) -> Result<(), String> {
    if obj.spec.backup_ref.trim().is_empty() {
        return Err("backupRef must not be empty".to_string());
    }
    if obj.spec.target_namespace.trim().is_empty() {
        return Err("targetNamespace must not be empty".to_string());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use proteus_crd::ProteusRestoreSpec;

    fn restore_with(backup_ref: &str, target_namespace: &str) -> ProteusRestore {
        ProteusRestore::new(
            "test",
            ProteusRestoreSpec {
                backup_ref: backup_ref.to_string(),
                backup_namespace: None,
                snapshot_id: None,
                target_namespace: target_namespace.to_string(),
                overwrite: false,
                include_resources: None,
            },
        )
    }

    #[test]
    fn validate_spec_rejects_empty_backup_ref() {
        let obj = restore_with("", "workloads");
        let err = validate_spec(&obj).expect_err("empty backupRef");
        assert!(err.contains("backupRef"));
    }

    #[test]
    fn validate_spec_rejects_empty_target_namespace() {
        let obj = restore_with("backup-1", "");
        let err = validate_spec(&obj).expect_err("empty targetNamespace");
        assert!(err.contains("targetNamespace"));
    }

    #[test]
    fn validate_spec_accepts_minimal_valid_spec() {
        let obj = restore_with("backup-1", "workloads");
        assert!(validate_spec(&obj).is_ok());
    }

    #[test]
    fn succeeded_restore_is_treated_as_terminal() {
        let mut obj = restore_with("backup-1", "workloads");
        obj.status = Some(ProteusRestoreStatus {
            phase: Some(RestorePhase::Succeeded),
            ..Default::default()
        });
        assert!(matches!(current_phase(&obj), Some(RestorePhase::Succeeded)));
    }

    #[test]
    fn terminal_status_on_success_sets_snapshot_and_completion() {
        let obj = restore_with("backup-1", "workloads");
        let status = terminal_status(&obj, Ok("deadbeef".to_string()));
        assert_eq!(status.phase, Some(RestorePhase::Succeeded));
        assert_eq!(status.restored_snapshot_id.as_deref(), Some("deadbeef"));
        assert_eq!(status.progress_percent, Some(100));
    }

    #[test]
    fn terminal_status_on_failure_preserves_previous_snapshot_id() {
        let mut obj = restore_with("backup-1", "workloads");
        obj.status = Some(ProteusRestoreStatus {
            restored_snapshot_id: Some("previous".to_string()),
            ..Default::default()
        });
        let status = terminal_status(&obj, Err("target PVC missing".to_string()));
        assert_eq!(status.phase, Some(RestorePhase::Failed));
        assert_eq!(status.restored_snapshot_id.as_deref(), Some("previous"));
        assert_eq!(status.message.as_deref(), Some("target PVC missing"));
    }

    #[test]
    fn status_changed_ignores_timestamp_only_on_same_failure() {
        let cur = ProteusRestoreStatus {
            phase: Some(RestorePhase::Failed),
            message: Some("pvc not empty".into()),
            completed_at: Some("t1".into()),
            ..Default::default()
        };
        let next = ProteusRestoreStatus {
            phase: Some(RestorePhase::Failed),
            message: Some("pvc not empty".into()),
            completed_at: Some("t2".into()),
            ..Default::default()
        };
        assert!(!status_changed(Some(&cur), &next));
    }

    #[test]
    fn status_changed_detects_phase_transition() {
        let cur = ProteusRestoreStatus {
            phase: Some(RestorePhase::Running),
            ..Default::default()
        };
        let next = ProteusRestoreStatus {
            phase: Some(RestorePhase::Failed),
            message: Some("boom".into()),
            ..Default::default()
        };
        assert!(status_changed(Some(&cur), &next));
    }
}

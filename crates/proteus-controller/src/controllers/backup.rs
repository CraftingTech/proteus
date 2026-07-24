use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use kube::api::{Patch, PatchParams};
use kube::runtime::controller::Action;
use kube::{Api, ResourceExt};
use proteus_crd::{BackupPhase, ProteusBackup, ProteusBackupStatus};
use tracing::{info, warn};

use super::ReconcileCtx;
use crate::error::ControllerResult;

pub async fn reconcile_backup(
    obj: Arc<ProteusBackup>,
    ctx: Arc<ReconcileCtx>,
) -> ControllerResult<Action> {
    let ns = obj.namespace().unwrap_or_else(|| "default".to_string());
    let name = obj.name_any();
    info!(%ns, %name, "reconciling ProteusBackup");

    let api: Api<ProteusBackup> = Api::namespaced(ctx.client.clone(), &ns);

    // Terminal phases never re-run the mount-pod pipeline (avoids flood + pod churn).
    // A new snapshot requires a new ProteusBackup object.
    if matches!(
        current_phase(&obj),
        Some(BackupPhase::Succeeded | BackupPhase::Failed)
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

    let outcome = crate::backup::run_backup(&obj, &ctx, &ns).await;
    let status = terminal_status(&obj, outcome);
    if status_changed(obj.status.as_ref(), &status) {
        // After Running patch, the in-memory obj may be stale; compare against intended terminal
        // fields only — always write the first terminal transition.
        patch_status(&api, &name, &status).await?;
    }

    refresh_counts(&ctx).await;

    let requeue = match status.phase {
        Some(BackupPhase::Succeeded | BackupPhase::Failed) => Duration::from_secs(3600),
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

fn current_phase(obj: &ProteusBackup) -> Option<BackupPhase> {
    obj.status.as_ref().and_then(|s| s.phase.clone())
}

/// Fields that survive a phase transition (history from the previous run).
fn carry_forward(obj: &ProteusBackup) -> ProteusBackupStatus {
    ProteusBackupStatus {
        last_snapshot_id: obj.status.as_ref().and_then(|s| s.last_snapshot_id.clone()),
        last_success_at: obj.status.as_ref().and_then(|s| s.last_success_at.clone()),
        last_failure_at: obj.status.as_ref().and_then(|s| s.last_failure_at.clone()),
        last_bytes: obj.status.as_ref().and_then(|s| s.last_bytes),
        retained_snapshots: obj.status.as_ref().and_then(|s| s.retained_snapshots),
        ..Default::default()
    }
}

fn running_status(obj: &ProteusBackup) -> ProteusBackupStatus {
    ProteusBackupStatus {
        phase: Some(BackupPhase::Running),
        message: Some("backup running: mounting PVCs and streaming to the repository".to_string()),
        ..carry_forward(obj)
    }
}

fn terminal_status(
    obj: &ProteusBackup,
    outcome: Result<(String, u64), String>,
) -> ProteusBackupStatus {
    match outcome {
        Ok((snapshot_id, bytes)) => ProteusBackupStatus {
            phase: Some(BackupPhase::Succeeded),
            message: Some(format!("backup succeeded ({bytes} bytes)")),
            last_snapshot_id: Some(snapshot_id),
            last_success_at: Some(Utc::now().to_rfc3339()),
            last_bytes: Some(bytes),
            ..carry_forward(obj)
        },
        Err(message) => ProteusBackupStatus {
            phase: Some(BackupPhase::Failed),
            message: Some(message),
            last_failure_at: Some(Utc::now().to_rfc3339()),
            ..carry_forward(obj)
        },
    }
}

/// Patch only when phase / stable message change — timestamps alone must not re-trigger.
fn status_changed(current: Option<&ProteusBackupStatus>, next: &ProteusBackupStatus) -> bool {
    match current {
        None => true,
        Some(cur) => {
            cur.phase != next.phase
                || message_fingerprint(cur.message.as_deref())
                    != message_fingerprint(next.message.as_deref())
                || cur.last_snapshot_id != next.last_snapshot_id
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
    api: &Api<ProteusBackup>,
    name: &str,
    status: &ProteusBackupStatus,
) -> ControllerResult<()> {
    let patch = serde_json::json!({ "status": status });
    api.patch_status(name, &PatchParams::default(), &Patch::Merge(&patch))
        .await?;
    Ok(())
}

fn validate_spec(obj: &ProteusBackup) -> Result<(), String> {
    if obj.spec.repository_ref.is_empty() {
        return Err("repositoryRef must not be empty".to_string());
    }
    if obj.spec.target_namespace.is_empty() {
        return Err("targetNamespace must not be empty".to_string());
    }
    if obj.spec.pvc_names.is_empty() {
        return Err("pvcNames must contain at least one PVC name".to_string());
    }
    if obj.spec.retention.keep_last == 0 {
        return Err("retention.keepLast must be >= 1".to_string());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use proteus_crd::{ProteusBackupSpec, RetentionPolicy};

    fn backup_with(pvc_names: Vec<String>) -> ProteusBackup {
        ProteusBackup::new(
            "test",
            ProteusBackupSpec {
                repository_ref: "repo".to_string(),
                repository_namespace: None,
                target_namespace: "workloads".to_string(),
                pvc_names,
                label_selector: None,
                schedule: None,
                retention: RetentionPolicy::default(),
                include_volumes: true,
                include_cluster_resources: false,
            },
        )
    }

    #[test]
    fn validate_spec_rejects_empty_pvc_names() {
        let obj = backup_with(vec![]);
        let err = validate_spec(&obj).expect_err("empty pvcNames");
        assert!(err.contains("pvcNames"));
    }

    #[test]
    fn validate_spec_accepts_at_least_one_pvc() {
        let obj = backup_with(vec!["data".to_string()]);
        assert!(validate_spec(&obj).is_ok());
    }

    #[test]
    fn succeeded_backup_is_treated_as_terminal() {
        let mut obj = backup_with(vec!["data".to_string()]);
        obj.status = Some(ProteusBackupStatus {
            phase: Some(BackupPhase::Succeeded),
            ..Default::default()
        });
        assert!(matches!(current_phase(&obj), Some(BackupPhase::Succeeded)));
    }

    #[test]
    fn terminal_status_on_success_sets_snapshot_and_bytes() {
        let obj = backup_with(vec!["data".to_string()]);
        let status = terminal_status(&obj, Ok(("deadbeef".to_string(), 1024)));
        assert_eq!(status.phase, Some(BackupPhase::Succeeded));
        assert_eq!(status.last_snapshot_id.as_deref(), Some("deadbeef"));
        assert_eq!(status.last_bytes, Some(1024));
    }

    #[test]
    fn terminal_status_on_failure_preserves_previous_snapshot_id() {
        let mut obj = backup_with(vec!["data".to_string()]);
        obj.status = Some(ProteusBackupStatus {
            last_snapshot_id: Some("previous".to_string()),
            ..Default::default()
        });
        let status = terminal_status(&obj, Err("mount pod failed".to_string()));
        assert_eq!(status.phase, Some(BackupPhase::Failed));
        assert_eq!(status.last_snapshot_id.as_deref(), Some("previous"));
        assert_eq!(status.message.as_deref(), Some("mount pod failed"));
    }

    #[test]
    fn status_changed_ignores_timestamp_only_on_same_failure() {
        let cur = ProteusBackupStatus {
            phase: Some(BackupPhase::Failed),
            message: Some("pvc missing".into()),
            last_failure_at: Some("t1".into()),
            ..Default::default()
        };
        let next = ProteusBackupStatus {
            phase: Some(BackupPhase::Failed),
            message: Some("pvc missing".into()),
            last_failure_at: Some("t2".into()),
            ..Default::default()
        };
        assert!(!status_changed(Some(&cur), &next));
    }

    #[test]
    fn status_changed_detects_phase_transition() {
        let cur = ProteusBackupStatus {
            phase: Some(BackupPhase::Running),
            message: Some("running".into()),
            ..Default::default()
        };
        let next = ProteusBackupStatus {
            phase: Some(BackupPhase::Failed),
            message: Some("boom".into()),
            ..Default::default()
        };
        assert!(status_changed(Some(&cur), &next));
    }
}

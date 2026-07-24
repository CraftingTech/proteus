use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use kube::api::{Patch, PatchParams};
use kube::runtime::controller::Action;
use kube::{Api, ResourceExt};
use proteus_crd::{BackupPhase, ProteusBackup, ProteusBackupStatus};
use tracing::{info, warn};

use super::ReconcileCtx;
use crate::backup::progress::BackupProgressSink;
use crate::error::ControllerResult;

pub async fn reconcile_backup(
    obj: Arc<ProteusBackup>,
    ctx: Arc<ReconcileCtx>,
) -> ControllerResult<Action> {
    let ns = obj.namespace().unwrap_or_else(|| "default".to_string());
    let name = obj.name_any();
    info!(%ns, %name, "reconciling ProteusBackup");

    let api: Api<ProteusBackup> = Api::namespaced(ctx.client.clone(), &ns);
    let active_key = format!("{ns}/{name}");

    if matches!(
        current_phase(&obj),
        Some(BackupPhase::Succeeded | BackupPhase::Failed)
    ) {
        // Best-effort: reap mount pods left after cancel/crash.
        let _ = crate::backup::pvc_reader::cleanup_backup_mount_pods(
            &ctx.client,
            &obj.spec.target_namespace,
            &name,
            &ns,
        )
        .await;
        refresh_counts(&ctx).await;
        return Ok(Action::requeue(Duration::from_secs(3600)));
    }

    {
        let active = ctx.active_backups.lock().unwrap_or_else(|e| e.into_inner());
        if active.contains(&active_key) {
            return Ok(Action::requeue(Duration::from_secs(5)));
        }
    }

    if let Err(message) = validate_spec(&obj) {
        let status = terminal_status(&obj, Err(message));
        if status_changed(obj.status.as_ref(), &status) {
            patch_status(&api, &name, &status).await?;
        }
        refresh_counts(&ctx).await;
        return Ok(Action::requeue(Duration::from_secs(3600)));
    }

    {
        let mut active = ctx.active_backups.lock().unwrap_or_else(|e| e.into_inner());
        active.insert(active_key.clone());
    }

    let running = running_status(&obj);
    if status_changed(obj.status.as_ref(), &running) {
        if let Err(err) = patch_status(&api, &name, &running).await {
            clear_active(&ctx, &active_key);
            return Err(err);
        }
    }

    let progress = Arc::new(BackupProgressSink::new(api.clone(), name.clone(), &obj));
    let outcome = crate::backup::run_backup(&obj, &ctx, &ns, progress).await;
    clear_active(&ctx, &active_key);

    let status = terminal_status(&obj, outcome);
    if status_changed(obj.status.as_ref(), &status) {
        patch_status(&api, &name, &status).await?;
    }

    refresh_counts(&ctx).await;

    let requeue = match status.phase {
        Some(BackupPhase::Succeeded | BackupPhase::Failed) => Duration::from_secs(3600),
        _ => Duration::from_secs(60),
    };
    Ok(Action::requeue(requeue))
}

fn clear_active(ctx: &ReconcileCtx, key: &str) {
    if let Ok(mut active) = ctx.active_backups.lock() {
        active.remove(key);
    }
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

fn carry_forward(obj: &ProteusBackup) -> ProteusBackupStatus {
    ProteusBackupStatus {
        last_snapshot_id: obj.status.as_ref().and_then(|s| s.last_snapshot_id.clone()),
        last_success_at: obj.status.as_ref().and_then(|s| s.last_success_at.clone()),
        last_failure_at: obj.status.as_ref().and_then(|s| s.last_failure_at.clone()),
        last_bytes: obj.status.as_ref().and_then(|s| s.last_bytes),
        retained_snapshots: obj.status.as_ref().and_then(|s| s.retained_snapshots),
        started_at: obj.status.as_ref().and_then(|s| s.started_at.clone()),
        duration_seconds: obj.status.as_ref().and_then(|s| s.duration_seconds),
        throughput_bytes_per_sec: obj.status.as_ref().and_then(|s| s.throughput_bytes_per_sec),
        progress_percent: obj.status.as_ref().and_then(|s| s.progress_percent),
        ..Default::default()
    }
}

fn running_status(obj: &ProteusBackup) -> ProteusBackupStatus {
    ProteusBackupStatus {
        phase: Some(BackupPhase::Running),
        message: Some("backup starting".to_string()),
        progress_percent: Some(0),
        started_at: Some(Utc::now().to_rfc3339()),
        ..carry_forward(obj)
    }
}

fn terminal_status(
    obj: &ProteusBackup,
    outcome: Result<(String, u64), String>,
) -> ProteusBackupStatus {
    match outcome {
        Ok((snapshot_id, bytes)) => {
            let started = obj
                .status
                .as_ref()
                .and_then(|s| s.started_at.as_deref())
                .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok());
            let duration_seconds = started.map(|s| {
                Utc::now()
                    .signed_duration_since(s.with_timezone(&Utc))
                    .num_seconds()
                    .max(1) as u64
            });
            let throughput_bytes_per_sec = duration_seconds.map(|d| bytes / d);
            ProteusBackupStatus {
                phase: Some(BackupPhase::Succeeded),
                message: Some(match (duration_seconds, throughput_bytes_per_sec) {
                    (Some(secs), Some(bps)) => {
                        format!("backup succeeded ({bytes} bytes in {secs}s, ~{bps} B/s)")
                    }
                    _ => format!("backup succeeded ({bytes} bytes)"),
                }),
                last_snapshot_id: Some(snapshot_id),
                last_success_at: Some(Utc::now().to_rfc3339()),
                last_bytes: Some(bytes),
                progress_percent: Some(100),
                duration_seconds,
                throughput_bytes_per_sec,
                ..carry_forward(obj)
            }
        }
        Err(message) => ProteusBackupStatus {
            phase: Some(BackupPhase::Failed),
            message: Some(message),
            last_failure_at: Some(Utc::now().to_rfc3339()),
            progress_percent: obj.status.as_ref().and_then(|s| s.progress_percent),
            ..carry_forward(obj)
        },
    }
}

fn status_changed(current: Option<&ProteusBackupStatus>, next: &ProteusBackupStatus) -> bool {
    match current {
        None => true,
        Some(cur) => {
            cur.phase != next.phase
                || cur.progress_percent != next.progress_percent
                || message_fingerprint(cur.message.as_deref())
                    != message_fingerprint(next.message.as_deref())
                || cur.last_snapshot_id != next.last_snapshot_id
                || cur.duration_seconds != next.duration_seconds
                || cur.throughput_bytes_per_sec != next.throughput_bytes_per_sec
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
                target_namespace: "default".to_string(),
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
    fn validate_rejects_empty_pvcs() {
        let b = backup_with(vec![]);
        assert!(validate_spec(&b).is_err());
    }

    #[test]
    fn validate_accepts_one_pvc() {
        let b = backup_with(vec!["data".into()]);
        assert!(validate_spec(&b).is_ok());
    }

    #[test]
    fn terminal_success_sets_snapshot() {
        let b = backup_with(vec!["data".into()]);
        let status = terminal_status(&b, Ok(("deadbeef".into(), 42)));
        assert_eq!(status.phase, Some(BackupPhase::Succeeded));
        assert_eq!(status.last_snapshot_id.as_deref(), Some("deadbeef"));
        assert_eq!(status.last_bytes, Some(42));
        assert_eq!(status.progress_percent, Some(100));
    }

    #[test]
    fn terminal_failure_keeps_prior_snapshot() {
        let mut b = backup_with(vec!["data".into()]);
        b.status = Some(ProteusBackupStatus {
            last_snapshot_id: Some("previous".to_string()),
            progress_percent: Some(40),
            ..Default::default()
        });
        let status = terminal_status(&b, Err("boom".into()));
        assert_eq!(status.phase, Some(BackupPhase::Failed));
        assert_eq!(status.last_snapshot_id.as_deref(), Some("previous"));
        assert_eq!(status.progress_percent, Some(40));
    }

    #[test]
    fn status_changed_detects_progress_bump() {
        let a = ProteusBackupStatus {
            phase: Some(BackupPhase::Running),
            progress_percent: Some(10),
            message: Some("reading".into()),
            ..Default::default()
        };
        let b = ProteusBackupStatus {
            phase: Some(BackupPhase::Running),
            progress_percent: Some(25),
            message: Some("reading".into()),
            ..Default::default()
        };
        assert!(status_changed(Some(&a), &b));
    }

    #[test]
    fn message_fingerprint_collapses_digits() {
        assert_eq!(
            message_fingerprint(Some("read 12 MiB / 100 MiB")),
            message_fingerprint(Some("read 99 MiB / 100 MiB"))
        );
    }
}

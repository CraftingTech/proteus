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
use crate::backup::recipe::{load_recipe, resolve_recipe, ResolveRecipeError};
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
        if let Ok(recipe) = load_recipe(&ctx.client, &obj, &ns).await {
            let _ = crate::backup::pvc_reader::cleanup_backup_mount_pods(
                &ctx.client,
                &recipe.target_namespace,
                &name,
                &ns,
            )
            .await;
        }
        refresh_counts(&ctx).await;
        return Ok(Action::requeue(Duration::from_secs(3600)));
    }

    {
        let active = ctx.active_backups.lock().unwrap_or_else(|e| e.into_inner());
        if active.contains(&active_key) {
            return Ok(Action::requeue(Duration::from_secs(5)));
        }
    }

    let recipe = match resolve_recipe(&ctx.client, &obj, &ns).await {
        Ok(recipe) => recipe,
        Err(ResolveRecipeError::NotReady(message)) => {
            let status = waiting_policy_status(&obj, &message);
            if status_changed(obj.status.as_ref(), &status) {
                patch_status(&api, &name, &status).await?;
            }
            refresh_counts(&ctx).await;
            return Ok(Action::requeue(Duration::from_secs(5)));
        }
        Err(ResolveRecipeError::Failed(message)) => {
            let status = terminal_status(&obj, Err(message), None);
            if status_changed(obj.status.as_ref(), &status) {
                patch_status(&api, &name, &status).await?;
            }
            refresh_counts(&ctx).await;
            return Ok(Action::requeue(Duration::from_secs(3600)));
        }
    };

    {
        let mut active = ctx.active_backups.lock().unwrap_or_else(|e| e.into_inner());
        active.insert(active_key.clone());
    }

    // Already handed to an agent: wait for the agent to patch a terminal status.
    if matches!(
        obj.status.as_ref().and_then(|s| s.data_plane.as_ref()),
        Some(proteus_crd::DataPlane::Agent)
    ) && matches!(current_phase(&obj), Some(BackupPhase::Running) | None)
    {
        clear_active(&ctx, &active_key);
        refresh_counts(&ctx).await;
        return Ok(Action::requeue(Duration::from_secs(5)));
    }

    let repo_kind = match crate::backup::repo::repository_kind(
        &ctx.client,
        &ns,
        &recipe.repository_ref,
        recipe.repository_namespace.as_deref(),
    )
    .await
    {
        Ok(kind) => kind,
        Err(message) => {
            let status = terminal_status(&obj, Err(message), None);
            if status_changed(obj.status.as_ref(), &status) {
                let _ = patch_status(&api, &name, &status).await;
            }
            clear_active(&ctx, &active_key);
            refresh_counts(&ctx).await;
            return Ok(Action::requeue(Duration::from_secs(3600)));
        }
    };

    let plane = match crate::data_plane::select_plane(
        &ctx.client,
        repo_kind,
        &recipe.target_namespace,
        &recipe.pvc_names,
    )
    .await
    {
        Ok(choice) => choice,
        Err(message) => {
            let status = terminal_status(&obj, Err(message), None);
            if status_changed(obj.status.as_ref(), &status) {
                let _ = patch_status(&api, &name, &status).await;
            }
            clear_active(&ctx, &active_key);
            refresh_counts(&ctx).await;
            return Ok(Action::requeue(Duration::from_secs(3600)));
        }
    };

    if let Some(node) = plane.assigned_node() {
        let mut running = running_status(&obj);
        running.data_plane = Some(plane.data_plane());
        running.assigned_node = Some(node.to_string());
        running.message = Some(format!(
            "assigned to proteus-node-agent on '{node}' (dataPlane=agent)"
        ));
        if status_changed(obj.status.as_ref(), &running) {
            if let Err(err) = patch_status(&api, &name, &running).await {
                clear_active(&ctx, &active_key);
                return Err(err);
            }
        }
        clear_active(&ctx, &active_key);
        refresh_counts(&ctx).await;
        return Ok(Action::requeue(Duration::from_secs(5)));
    }

    let running = {
        let mut status = running_status(&obj);
        status.data_plane = Some(plane.data_plane());
        if let crate::data_plane::PlaneChoice::Exec { reason } = &plane {
            status.message = Some(format!("backup starting ({reason})"));
        }
        status
    };
    // Capture now: `obj` is the reconcile snapshot and will not see this patch.
    let run_started_at = running.started_at.clone();
    if status_changed(obj.status.as_ref(), &running) {
        if let Err(err) = patch_status(&api, &name, &running).await {
            clear_active(&ctx, &active_key);
            return Err(err);
        }
    }

    let progress = Arc::new(BackupProgressSink::new(api.clone(), name.clone(), &obj));
    let outcome = crate::backup::run_backup(&obj, &recipe, &ctx, &ns, progress).await;
    clear_active(&ctx, &active_key);

    let mut status = terminal_status(&obj, outcome, run_started_at.as_deref());
    status.data_plane = Some(proteus_crd::DataPlane::Exec);
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
        data_plane: obj.status.as_ref().and_then(|s| s.data_plane.clone()),
        assigned_node: obj.status.as_ref().and_then(|s| s.assigned_node.clone()),
        ..Default::default()
    }
}

fn waiting_policy_status(obj: &ProteusBackup, message: &str) -> ProteusBackupStatus {
    ProteusBackupStatus {
        phase: Some(BackupPhase::Pending),
        message: Some(message.to_string()),
        progress_percent: Some(0),
        ..carry_forward(obj)
    }
}

fn running_status(obj: &ProteusBackup) -> ProteusBackupStatus {
    // Resume wall-clock if we are already Running (controller restart mid-backup).
    let started_at = obj
        .status
        .as_ref()
        .filter(|s| s.phase == Some(BackupPhase::Running))
        .and_then(|s| s.started_at.clone())
        .unwrap_or_else(|| Utc::now().to_rfc3339());
    ProteusBackupStatus {
        phase: Some(BackupPhase::Running),
        message: Some("backup starting".to_string()),
        progress_percent: Some(0),
        started_at: Some(started_at),
        // Do not surface the previous run's timing while this run is in progress.
        duration_seconds: None,
        throughput_bytes_per_sec: None,
        ..carry_forward(obj)
    }
}

/// Wall-clock duration and approximate B/s from `started_at` → `finished_at`.
///
/// Duration is clamped to ≥ 1s so sub-second backups still report a throughput.
fn compute_throughput(
    started_at: Option<&str>,
    bytes: u64,
    finished_at: chrono::DateTime<Utc>,
) -> (Option<u64>, Option<u64>) {
    let duration_seconds = started_at
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|s| {
            finished_at
                .signed_duration_since(s.with_timezone(&Utc))
                .num_seconds()
                .max(1) as u64
        });
    let throughput_bytes_per_sec = duration_seconds.map(|d| bytes / d);
    (duration_seconds, throughput_bytes_per_sec)
}

fn terminal_status(
    obj: &ProteusBackup,
    outcome: Result<(String, u64), String>,
    run_started_at: Option<&str>,
) -> ProteusBackupStatus {
    match outcome {
        Ok((snapshot_id, bytes)) => {
            let finished_at = Utc::now();
            let (duration_seconds, throughput_bytes_per_sec) =
                compute_throughput(run_started_at, bytes, finished_at);
            ProteusBackupStatus {
                phase: Some(BackupPhase::Succeeded),
                message: Some(match (duration_seconds, throughput_bytes_per_sec) {
                    (Some(secs), Some(bps)) => {
                        format!("backup succeeded ({bytes} bytes in {secs}s, ~{bps} B/s)")
                    }
                    _ => format!("backup succeeded ({bytes} bytes)"),
                }),
                last_snapshot_id: Some(snapshot_id),
                last_success_at: Some(finished_at.to_rfc3339()),
                last_bytes: Some(bytes),
                progress_percent: Some(100),
                started_at: run_started_at.map(str::to_string),
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
            started_at: run_started_at
                .map(str::to_string)
                .or_else(|| obj.status.as_ref().and_then(|s| s.started_at.clone())),
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
                || cur.data_plane != next.data_plane
                || cur.assigned_node != next.assigned_node
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backup::recipe::BackupRecipe;
    use proteus_crd::{ProteusBackupSpec, RetentionPolicy};

    fn backup_with(pvc_names: Vec<String>) -> ProteusBackup {
        ProteusBackup::new(
            "test",
            ProteusBackupSpec {
                policy_ref: None,
                policy_namespace: None,
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
    fn waiting_policy_keeps_pending_not_failed() {
        let b = backup_with(vec!["data".into()]);
        let status = waiting_policy_status(&b, "waiting for policy");
        assert_eq!(status.phase, Some(BackupPhase::Pending));
        assert_eq!(status.message.as_deref(), Some("waiting for policy"));
    }

    #[test]
    fn inline_recipe_validates() {
        let b = backup_with(vec!["data".into()]);
        assert!(BackupRecipe::from_inline(&b.spec).validate().is_ok());
    }

    #[test]
    fn inline_recipe_rejects_empty_pvcs() {
        let b = backup_with(vec![]);
        assert!(BackupRecipe::from_inline(&b.spec).validate().is_err());
    }

    #[test]
    fn terminal_success_sets_snapshot() {
        let b = backup_with(vec!["data".into()]);
        let started = (Utc::now() - chrono::Duration::seconds(10)).to_rfc3339();
        let status = terminal_status(&b, Ok(("deadbeef".into(), 42)), Some(&started));
        assert_eq!(status.phase, Some(BackupPhase::Succeeded));
        assert_eq!(status.last_snapshot_id.as_deref(), Some("deadbeef"));
        assert_eq!(status.last_bytes, Some(42));
        assert_eq!(status.progress_percent, Some(100));
        assert_eq!(status.started_at.as_deref(), Some(started.as_str()));
        assert!(status.duration_seconds.is_some_and(|d| d >= 10));
        assert!(status.throughput_bytes_per_sec.is_some());
    }

    #[test]
    fn terminal_success_records_throughput_from_run_start_not_stale_status() {
        // Reconcile `obj` often still has the pre-Running status (no startedAt).
        let b = backup_with(vec!["data".into()]);
        let started = (Utc::now() - chrono::Duration::seconds(5)).to_rfc3339();
        let status = terminal_status(&b, Ok(("abc".into(), 50_000)), Some(&started));
        assert_eq!(status.duration_seconds, Some(5));
        assert_eq!(status.throughput_bytes_per_sec, Some(10_000));
    }

    #[test]
    fn compute_throughput_clamps_subsecond_duration() {
        let started = Utc::now().to_rfc3339();
        let finished = Utc::now();
        let (secs, bps) = compute_throughput(Some(&started), 4_096, finished);
        assert_eq!(secs, Some(1));
        assert_eq!(bps, Some(4_096));
    }

    #[test]
    fn compute_throughput_none_without_started_at() {
        let (secs, bps) = compute_throughput(None, 100, Utc::now());
        assert_eq!(secs, None);
        assert_eq!(bps, None);
    }

    #[test]
    fn running_status_preserves_started_at_when_already_running() {
        let mut b = backup_with(vec!["data".into()]);
        let prior = "2026-07-01T12:00:00Z".to_string();
        b.status = Some(ProteusBackupStatus {
            phase: Some(BackupPhase::Running),
            started_at: Some(prior.clone()),
            duration_seconds: Some(99),
            throughput_bytes_per_sec: Some(1),
            ..Default::default()
        });
        let running = running_status(&b);
        assert_eq!(running.started_at.as_deref(), Some(prior.as_str()));
        assert_eq!(running.duration_seconds, None);
        assert_eq!(running.throughput_bytes_per_sec, None);
    }

    #[test]
    fn terminal_failure_keeps_prior_snapshot() {
        let mut b = backup_with(vec!["data".into()]);
        b.status = Some(ProteusBackupStatus {
            last_snapshot_id: Some("previous".to_string()),
            progress_percent: Some(40),
            ..Default::default()
        });
        let status = terminal_status(&b, Err("boom".into()), None);
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

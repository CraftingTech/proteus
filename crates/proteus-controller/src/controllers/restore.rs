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
        let status = terminal_status(&obj, Err(message), None, None);
        if status_changed(obj.status.as_ref(), &status) {
            patch_status(&api, &name, &status).await?;
        }
        refresh_counts(&ctx).await;
        return Ok(Action::requeue(Duration::from_secs(3600)));
    }

    // Agent-owned restore: wait for the node-agent to finish.
    if matches!(
        obj.status.as_ref().and_then(|s| s.data_plane.as_ref()),
        Some(proteus_crd::DataPlane::Agent)
    ) && matches!(current_phase(&obj), Some(RestorePhase::Running) | None)
    {
        refresh_counts(&ctx).await;
        return Ok(Action::requeue(Duration::from_secs(5)));
    }

    let (repo_kind, pvc_namespace, pvc_names) = match restore_plane_inputs(&ctx, &obj, &ns).await {
        Ok(inputs) => inputs,
        Err(message) => {
            let status = terminal_status(&obj, Err(message), None, None);
            if status_changed(obj.status.as_ref(), &status) {
                patch_status(&api, &name, &status).await?;
            }
            refresh_counts(&ctx).await;
            return Ok(Action::requeue(Duration::from_secs(3600)));
        }
    };

    let plane =
        match crate::data_plane::select_plane(&ctx.client, repo_kind, &pvc_namespace, &pvc_names)
            .await
        {
            Ok(choice) => choice,
            Err(message) => {
                let status = terminal_status(&obj, Err(message), None, None);
                if status_changed(obj.status.as_ref(), &status) {
                    patch_status(&api, &name, &status).await?;
                }
                refresh_counts(&ctx).await;
                return Ok(Action::requeue(Duration::from_secs(3600)));
            }
        };

    if let Some(node) = plane.assigned_node() {
        if let Err(err) = crate::agent::ensure_mover_identity(&ctx.client, &pvc_namespace).await {
            let status = terminal_status(
                &obj,
                Err(format!("failed to provision mover identity: {err}")),
                None,
                None,
            );
            if status_changed(obj.status.as_ref(), &status) {
                patch_status(&api, &name, &status).await?;
            }
            refresh_counts(&ctx).await;
            return Ok(Action::requeue(Duration::from_secs(30)));
        }
        let mut running = running_status(&obj);
        running.data_plane = Some(plane.data_plane());
        running.assigned_node = Some(node.to_string());
        running.message = Some(format!(
            "assigned to proteus-node-agent on '{node}' (dataPlane=agent)"
        ));
        if status_changed(obj.status.as_ref(), &running) {
            patch_status(&api, &name, &running).await?;
        }
        refresh_counts(&ctx).await;
        return Ok(Action::requeue(Duration::from_secs(5)));
    }

    let running = {
        let mut status = running_status(&obj);
        status.data_plane = Some(plane.data_plane());
        if let crate::data_plane::PlaneChoice::Exec { reason } = &plane {
            status.message = Some(format!("restore running ({reason})"));
        }
        status
    };
    let run_started_at = running.started_at.clone();
    if status_changed(obj.status.as_ref(), &running) {
        patch_status(&api, &name, &running).await?;
    }

    let outcome = crate::restore::run_restore(&obj, &ctx, &ns).await;
    let mut status = terminal_status(&obj, outcome, run_started_at.as_deref(), None);
    status.data_plane = Some(proteus_crd::DataPlane::Exec);
    if status_changed(obj.status.as_ref(), &status) {
        patch_status(&api, &name, &status).await?;
    }

    refresh_counts(&ctx).await;

    let requeue = match status.phase {
        Some(RestorePhase::Succeeded | RestorePhase::Failed) => Duration::from_secs(3600),
        _ => Duration::from_secs(60),
    };
    Ok(Action::requeue(requeue))
}

async fn restore_plane_inputs(
    ctx: &ReconcileCtx,
    restore: &ProteusRestore,
    restore_namespace: &str,
) -> Result<(crate::data_plane::RepositoryKind, String, Vec<String>), String> {
    let backup = crate::restore::resolve_backup_for_plane(ctx, restore, restore_namespace).await?;
    let backup_namespace = backup
        .namespace()
        .unwrap_or_else(|| restore_namespace.to_string());
    let recipe =
        crate::backup::recipe::load_recipe(&ctx.client, &backup, &backup_namespace).await?;
    let kind = crate::backup::repo::repository_kind(
        &ctx.client,
        &backup_namespace,
        &recipe.repository_ref,
        recipe.repository_namespace.as_deref(),
    )
    .await?;
    Ok((
        kind,
        restore.spec.target_namespace.clone(),
        recipe.pvc_names,
    ))
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
    let started_at = obj
        .status
        .as_ref()
        .filter(|s| s.phase == Some(RestorePhase::Running))
        .and_then(|s| s.started_at.clone())
        .unwrap_or_else(|| Utc::now().to_rfc3339());
    ProteusRestoreStatus {
        phase: Some(RestorePhase::Running),
        message: Some("restore running: resolving snapshot and writing target PVCs".to_string()),
        restored_snapshot_id: obj
            .status
            .as_ref()
            .and_then(|s| s.restored_snapshot_id.clone()),
        progress_percent: Some(0),
        completed_at: None,
        started_at: Some(started_at),
        last_bytes: None,
        duration_seconds: None,
        throughput_bytes_per_sec: None,
        data_plane: obj.status.as_ref().and_then(|s| s.data_plane.clone()),
        assigned_node: obj.status.as_ref().and_then(|s| s.assigned_node.clone()),
    }
}

fn terminal_status(
    obj: &ProteusRestore,
    outcome: Result<String, String>,
    run_started_at: Option<&str>,
    last_bytes: Option<u64>,
) -> ProteusRestoreStatus {
    let finished_at = Utc::now();
    match outcome {
        Ok(snapshot_id) => {
            let bytes = last_bytes.unwrap_or(0);
            let (duration_seconds, throughput_bytes_per_sec) =
                compute_restore_throughput(run_started_at, bytes, finished_at);
            ProteusRestoreStatus {
                phase: Some(RestorePhase::Succeeded),
                message: Some(format!("restore succeeded from snapshot {snapshot_id}")),
                restored_snapshot_id: Some(snapshot_id),
                progress_percent: Some(100),
                completed_at: Some(finished_at.to_rfc3339()),
                started_at: run_started_at.map(str::to_string),
                last_bytes: last_bytes.or(Some(0)),
                duration_seconds,
                throughput_bytes_per_sec,
                data_plane: obj.status.as_ref().and_then(|s| s.data_plane.clone()),
                assigned_node: obj.status.as_ref().and_then(|s| s.assigned_node.clone()),
            }
        }
        Err(message) => ProteusRestoreStatus {
            phase: Some(RestorePhase::Failed),
            message: Some(message),
            restored_snapshot_id: obj
                .status
                .as_ref()
                .and_then(|s| s.restored_snapshot_id.clone()),
            progress_percent: obj.status.as_ref().and_then(|s| s.progress_percent),
            completed_at: Some(finished_at.to_rfc3339()),
            started_at: run_started_at
                .map(str::to_string)
                .or_else(|| obj.status.as_ref().and_then(|s| s.started_at.clone())),
            last_bytes: obj.status.as_ref().and_then(|s| s.last_bytes),
            duration_seconds: None,
            throughput_bytes_per_sec: None,
            data_plane: obj.status.as_ref().and_then(|s| s.data_plane.clone()),
            assigned_node: obj.status.as_ref().and_then(|s| s.assigned_node.clone()),
        },
    }
}

fn compute_restore_throughput(
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

/// Patch only when phase / stable message change — timestamps alone must not re-trigger.
fn status_changed(current: Option<&ProteusRestoreStatus>, next: &ProteusRestoreStatus) -> bool {
    match current {
        None => true,
        Some(cur) => {
            cur.phase != next.phase
                || message_fingerprint(cur.message.as_deref())
                    != message_fingerprint(next.message.as_deref())
                || cur.restored_snapshot_id != next.restored_snapshot_id
                || cur.data_plane != next.data_plane
                || cur.assigned_node != next.assigned_node
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
        let status = terminal_status(&obj, Ok("deadbeef".to_string()), None, Some(42));
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
        let status = terminal_status(&obj, Err("target PVC missing".to_string()), None, None);
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

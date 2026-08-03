use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
use kube::api::{DeleteParams, ListParams, Patch, PatchParams, PostParams};
use kube::runtime::controller::Action;
use kube::{Api, Resource, ResourceExt};
use proteus_api::gc_repository_after_backup_delete;
use proteus_core::{backup_run_name, last_tick_at_or_before, next_run_after, schedule_is_due};
use proteus_crd::{
    BackupPhase, BackupPolicyPhase, ProteusBackup, ProteusBackupPolicy, ProteusBackupPolicyStatus,
    ProteusBackupSpec,
};
use tracing::{info, warn};

use super::ReconcileCtx;
use crate::backup::recipe::validate_policy_spec;
use crate::backup::retention::{select_prunable, SucceededRunRef};
use crate::error::ControllerResult;

const MAX_REQUEUE_SECS: u64 = 300;
const ACTIVE_RUN_REQUEUE_SECS: u64 = 30;

pub async fn reconcile_backup_policy(
    obj: Arc<ProteusBackupPolicy>,
    ctx: Arc<ReconcileCtx>,
) -> ControllerResult<Action> {
    let ns = obj.namespace().unwrap_or_else(|| "default".to_string());
    let name = obj.name_any();
    info!(%ns, %name, "reconciling ProteusBackupPolicy");

    let api: Api<ProteusBackupPolicy> = Api::namespaced(ctx.client.clone(), &ns);
    let prev = obj.status.clone().unwrap_or_default();

    let mut status = match validate_policy_spec(&obj) {
        Ok(()) => ProteusBackupPolicyStatus {
            phase: Some(BackupPolicyPhase::Ready),
            message: Some("policy is valid".to_string()),
            next_run_at: prev.next_run_at.clone(),
            last_schedule_time: prev.last_schedule_time.clone(),
            last_run_name: prev.last_run_name.clone(),
        },
        Err(message) => ProteusBackupPolicyStatus {
            phase: Some(BackupPolicyPhase::Invalid),
            message: Some(message),
            next_run_at: None,
            last_schedule_time: prev.last_schedule_time.clone(),
            last_run_name: prev.last_run_name.clone(),
        },
    };

    let mut requeue_secs = MAX_REQUEUE_SECS;

    if status.phase == Some(BackupPolicyPhase::Ready) {
        if let Err(err) = prune_keep_last(&obj, &ns, &name, &ctx).await {
            warn!(%ns, %name, error = %err, "keepLast prune failed");
            status.message = Some(format!("policy is valid; prune warning: {err}"));
        }

        match reconcile_schedule(&obj, &ns, &name, &mut status, &ctx).await {
            Ok(secs) => requeue_secs = secs,
            Err(err) => {
                // Operational failures (apiserver blips, create conflicts, etc.) must not
                // mark a valid recipe Invalid — that blocks Run now until the next Ready.
                warn!(%ns, %name, error = %err, "schedule reconcile failed; staying Ready");
                status.phase = Some(BackupPolicyPhase::Ready);
                status.message = Some(format!("policy is valid; schedule warning: {err}"));
                requeue_secs = ACTIVE_RUN_REQUEUE_SECS;
            }
        }
    }

    if status_changed(obj.status.as_ref(), &status) {
        let patch = serde_json::json!({ "status": status });
        api.patch_status(&name, &PatchParams::default(), &Patch::Merge(&patch))
            .await?;
    }

    if let Err(err) = ctx.api_state.refresh_counts().await {
        warn!(error = %err, "failed to refresh cluster snapshot counts");
    } else {
        ctx.api_state.mark_reconciled();
    }

    Ok(Action::requeue(Duration::from_secs(requeue_secs.max(5))))
}

async fn reconcile_schedule(
    policy: &ProteusBackupPolicy,
    policy_ns: &str,
    policy_name: &str,
    status: &mut ProteusBackupPolicyStatus,
    ctx: &ReconcileCtx,
) -> Result<u64, String> {
    let schedule = policy
        .spec
        .schedule
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());

    let Some(schedule) = schedule else {
        status.next_run_at = None;
        return Ok(MAX_REQUEUE_SECS);
    };

    if policy.spec.paused {
        status.next_run_at = None;
        if status.message.as_deref() == Some("policy is valid") {
            status.message = Some("policy is valid (schedule paused)".to_string());
        }
        return Ok(MAX_REQUEUE_SECS);
    }

    let now = Utc::now();
    let next_parsed = status
        .next_run_at
        .as_deref()
        .map(parse_rfc3339)
        .transpose()?;

    if schedule_is_due(now, next_parsed) {
        if policy_has_active_run(ctx, policy_ns, policy_name).await? {
            status.message = Some(
                "scheduled tick deferred: a non-terminal run already exists for this policy"
                    .to_string(),
            );
            return Ok(ACTIVE_RUN_REQUEUE_SECS);
        }

        let from = status
            .last_schedule_time
            .as_deref()
            .map(parse_rfc3339)
            .transpose()?
            .or(next_parsed.map(|t| t - chrono::Duration::seconds(1)))
            .unwrap_or(now - chrono::Duration::seconds(1));

        let tick = last_tick_at_or_before(schedule, from, now)?
            .or(next_parsed)
            .unwrap_or(now);

        let run_name = spawn_scheduled_run(policy, policy_ns, policy_name, tick, ctx).await?;
        status.last_schedule_time = Some(tick.to_rfc3339());
        status.last_run_name = Some(run_name);
        status.message = Some("policy is valid".to_string());
    }

    let after = status
        .last_schedule_time
        .as_deref()
        .map(parse_rfc3339)
        .transpose()?
        .unwrap_or(now);
    let next = next_run_after(schedule, after)?;
    status.next_run_at = Some(next.to_rfc3339());

    let until = (next - now).num_seconds().max(0) as u64;
    Ok(until.clamp(5, MAX_REQUEUE_SECS))
}

async fn spawn_scheduled_run(
    policy: &ProteusBackupPolicy,
    policy_ns: &str,
    policy_name: &str,
    tick: DateTime<Utc>,
    ctx: &ReconcileCtx,
) -> Result<String, String> {
    let run_name = backup_run_name(policy_name, tick);
    let backup = ProteusBackup {
        metadata: ObjectMeta {
            name: Some(run_name.clone()),
            namespace: Some(policy_ns.to_string()),
            ..ObjectMeta::default()
        },
        spec: ProteusBackupSpec {
            policy_ref: Some(policy_name.to_string()),
            policy_namespace: Some(policy_ns.to_string()),
            repository_ref: policy.spec.repository_ref.clone(),
            repository_namespace: policy.spec.repository_namespace.clone(),
            target_namespace: policy.spec.target_namespace.clone(),
            pvc_names: policy.spec.pvc_names.clone(),
            label_selector: None,
            schedule: None,
            retention: policy.spec.retention.clone(),
            include_volumes: policy.spec.include_volumes,
            include_cluster_resources: policy.spec.include_cluster_resources,
        },
        status: None,
    };

    let api: Api<ProteusBackup> = Api::namespaced(ctx.client.clone(), policy_ns);
    match api.create(&PostParams::default(), &backup).await {
        Ok(created) => {
            info!(
                %policy_ns,
                policy = %policy_name,
                run = %created.name_any(),
                "spawned scheduled backup run"
            );
            Ok(created.name_any())
        }
        Err(kube::Error::Api(err)) if err.code == 409 => {
            // Idempotent: a prior reconcile already created this stamp.
            Ok(run_name)
        }
        Err(err) => Err(format!(
            "failed to create scheduled run '{run_name}': {err}"
        )),
    }
}

async fn policy_has_active_run(
    ctx: &ReconcileCtx,
    policy_ns: &str,
    policy_name: &str,
) -> Result<bool, String> {
    let api: Api<ProteusBackup> = Api::all(ctx.client.clone());
    let list = api
        .list(&ListParams::default())
        .await
        .map_err(|err| format!("failed to list backups: {err}"))?;

    Ok(list.items.iter().any(|b| {
        if !backup_matches_policy(b, policy_ns, policy_name) {
            return false;
        }
        !is_terminal_phase(b.status.as_ref().and_then(|s| s.phase.as_ref()))
    }))
}

fn backup_matches_policy(backup: &ProteusBackup, policy_ns: &str, policy_name: &str) -> bool {
    let Some(pref) = backup
        .spec
        .policy_ref
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    else {
        return false;
    };
    if pref != policy_name {
        return false;
    }
    let backup_ns = backup.namespace().unwrap_or_else(|| "default".to_string());
    let pns = backup
        .spec
        .policy_namespace
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(backup_ns.as_str());
    pns == policy_ns
}

fn is_terminal_phase(phase: Option<&BackupPhase>) -> bool {
    matches!(
        phase,
        Some(BackupPhase::Succeeded) | Some(BackupPhase::Failed)
    )
}

async fn prune_keep_last(
    policy: &ProteusBackupPolicy,
    policy_ns: &str,
    policy_name: &str,
    ctx: &ReconcileCtx,
) -> Result<(), String> {
    let keep_last = policy.spec.retention.keep_last;
    let api_all: Api<ProteusBackup> = Api::all(ctx.client.clone());
    let list = api_all
        .list(&ListParams::default())
        .await
        .map_err(|err| format!("failed to list backups for prune: {err}"))?;

    let succeeded: Vec<SucceededRunRef> = list
        .items
        .iter()
        .filter(|b| backup_matches_policy(b, policy_ns, policy_name))
        .filter(|b| {
            matches!(
                b.status.as_ref().and_then(|s| s.phase.as_ref()),
                Some(BackupPhase::Succeeded)
            )
        })
        .map(|b| {
            let sort_key = b
                .status
                .as_ref()
                .and_then(|s| s.last_success_at.clone())
                .or_else(|| {
                    b.meta()
                        .creation_timestamp
                        .as_ref()
                        .map(|t| t.0.to_rfc3339())
                })
                .unwrap_or_default();
            SucceededRunRef {
                namespace: b.namespace().unwrap_or_else(|| "default".to_string()),
                name: b.name_any(),
                sort_key,
            }
        })
        .collect();

    let prunable = select_prunable(succeeded, keep_last);
    for run in prunable {
        delete_run_with_gc(ctx, &run.namespace, &run.name, policy).await?;
    }
    Ok(())
}

async fn delete_run_with_gc(
    ctx: &ReconcileCtx,
    namespace: &str,
    name: &str,
    policy: &ProteusBackupPolicy,
) -> Result<(), String> {
    let repo_ns = policy
        .spec
        .repository_namespace
        .as_deref()
        .unwrap_or(namespace);
    let repo_ref = policy.spec.repository_ref.as_str();

    match gc_repository_after_backup_delete(&ctx.api_state, namespace, name, repo_ns, repo_ref)
        .await
    {
        Ok(removed) => {
            info!(%namespace, %name, removed, "GC before keepLast delete");
        }
        Err(err) => {
            return Err(format!(
                "GC failed for '{namespace}/{name}'; leaving CR in place: {err}"
            ));
        }
    }

    let api: Api<ProteusBackup> = Api::namespaced(ctx.client.clone(), namespace);
    api.delete(name, &DeleteParams::default())
        .await
        .map_err(|err| format!("failed to delete pruned run '{namespace}/{name}': {err}"))?;
    info!(%namespace, %name, "pruned Succeeded run (keepLast)");
    Ok(())
}

fn parse_rfc3339(value: &str) -> Result<DateTime<Utc>, String> {
    DateTime::parse_from_rfc3339(value.trim())
        .map(|dt| dt.with_timezone(&Utc))
        .map_err(|err| format!("invalid RFC3339 timestamp '{value}': {err}"))
}

fn status_changed(
    current: Option<&ProteusBackupPolicyStatus>,
    next: &ProteusBackupPolicyStatus,
) -> bool {
    match current {
        None => true,
        Some(cur) => {
            cur.phase != next.phase
                || cur.message != next.message
                || cur.next_run_at != next.next_run_at
                || cur.last_schedule_time != next.last_schedule_time
                || cur.last_run_name != next.last_run_name
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proteus_crd::{ProteusBackupPolicySpec, RetentionPolicy};

    fn policy(pvc_names: Vec<String>) -> ProteusBackupPolicy {
        ProteusBackupPolicy::new(
            "nightly",
            ProteusBackupPolicySpec {
                repository_ref: "repo".into(),
                repository_namespace: None,
                target_namespace: "default".into(),
                pvc_names,
                label_selector: None,
                schedule: None,
                paused: false,
                retention: RetentionPolicy::default(),
                include_volumes: true,
                include_cluster_resources: false,
            },
        )
    }

    #[test]
    fn status_changed_on_phase() {
        let a = ProteusBackupPolicyStatus {
            phase: Some(BackupPolicyPhase::Invalid),
            message: Some("x".into()),
            ..Default::default()
        };
        let b = ProteusBackupPolicyStatus {
            phase: Some(BackupPolicyPhase::Ready),
            message: Some("x".into()),
            ..Default::default()
        };
        assert!(status_changed(Some(&a), &b));
    }

    #[test]
    fn status_changed_on_next_run() {
        let a = ProteusBackupPolicyStatus {
            phase: Some(BackupPolicyPhase::Ready),
            next_run_at: Some("2026-01-01T00:00:00Z".into()),
            ..Default::default()
        };
        let b = ProteusBackupPolicyStatus {
            phase: Some(BackupPolicyPhase::Ready),
            next_run_at: Some("2026-01-02T00:00:00Z".into()),
            ..Default::default()
        };
        assert!(status_changed(Some(&a), &b));
    }

    #[test]
    fn validate_accepts_ready_recipe() {
        assert!(validate_policy_spec(&policy(vec!["data".into()])).is_ok());
    }

    #[test]
    fn validate_rejects_bad_cron() {
        let mut p = policy(vec!["data".into()]);
        p.spec.schedule = Some("not-a-cron".into());
        assert!(validate_policy_spec(&p).is_err());
    }

    #[test]
    fn validate_accepts_five_field_cron() {
        let mut p = policy(vec!["data".into()]);
        p.spec.schedule = Some("0 2 * * *".into());
        assert!(validate_policy_spec(&p).is_ok());
    }
}

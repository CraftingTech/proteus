use std::sync::Arc;
use std::time::Duration;

use kube::api::{Patch, PatchParams};
use kube::runtime::controller::Action;
use kube::{Api, ResourceExt};
use proteus_crd::{BackupPolicyPhase, ProteusBackupPolicy, ProteusBackupPolicyStatus};
use tracing::{info, warn};

use super::ReconcileCtx;
use crate::backup::recipe::validate_policy_spec;
use crate::error::ControllerResult;

pub async fn reconcile_backup_policy(
    obj: Arc<ProteusBackupPolicy>,
    ctx: Arc<ReconcileCtx>,
) -> ControllerResult<Action> {
    let ns = obj.namespace().unwrap_or_else(|| "default".to_string());
    let name = obj.name_any();
    info!(%ns, %name, "reconciling ProteusBackupPolicy");

    let api: Api<ProteusBackupPolicy> = Api::namespaced(ctx.client.clone(), &ns);
    let status = match validate_policy_spec(&obj) {
        Ok(()) => ProteusBackupPolicyStatus {
            phase: Some(BackupPolicyPhase::Ready),
            message: Some("policy is valid".to_string()),
        },
        Err(message) => ProteusBackupPolicyStatus {
            phase: Some(BackupPolicyPhase::Invalid),
            message: Some(message),
        },
    };

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

    Ok(Action::requeue(Duration::from_secs(300)))
}

fn status_changed(
    current: Option<&ProteusBackupPolicyStatus>,
    next: &ProteusBackupPolicyStatus,
) -> bool {
    match current {
        None => true,
        Some(cur) => cur.phase != next.phase || cur.message != next.message,
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
        };
        let b = ProteusBackupPolicyStatus {
            phase: Some(BackupPolicyPhase::Ready),
            message: Some("x".into()),
        };
        assert!(status_changed(Some(&a), &b));
    }

    #[test]
    fn validate_accepts_ready_recipe() {
        assert!(validate_policy_spec(&policy(vec!["data".into()])).is_ok());
    }
}

use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use kube::api::{Patch, PatchParams};
use kube::runtime::controller::Action;
use kube::{Api, ResourceExt};
use proteus_crd::{ProteusRepository, ProteusRepositoryStatus, RepositoryBackend, RepositoryPhase};
use tracing::info;

use super::ReconcileCtx;
use crate::error::{ControllerError, ControllerResult};

pub async fn reconcile_repository(
    obj: Arc<ProteusRepository>,
    ctx: Arc<ReconcileCtx>,
) -> ControllerResult<Action> {
    let ns = match obj.namespace() {
        Some(ns) => ns,
        None => "default".to_string(),
    };
    let name = obj.name_any();
    info!(%ns, %name, "reconciling ProteusRepository");

    validate_spec(&obj)?;

    let api: Api<ProteusRepository> = Api::namespaced(ctx.client.clone(), &ns);
    let status = ProteusRepositoryStatus {
        phase: Some(RepositoryPhase::Ready),
        message: Some("repository accepted".to_string()),
        object_count: obj.status.as_ref().and_then(|s| s.object_count),
        bytes_stored: obj.status.as_ref().and_then(|s| s.bytes_stored),
        last_checked_at: Some(Utc::now().to_rfc3339()),
    };

    let patch = serde_json::json!({ "status": status });
    api.patch_status(&name, &PatchParams::default(), &Patch::Merge(&patch))
        .await?;

    if let Err(err) = ctx.api_state.refresh_counts().await {
        tracing::warn!(error = %err, "failed to refresh cluster snapshot counts");
        ctx.api_state.mark_reconciled();
    }
    Ok(Action::requeue(Duration::from_secs(300)))
}

fn validate_spec(obj: &ProteusRepository) -> ControllerResult<()> {
    match &obj.spec.backend {
        RepositoryBackend::Local(local) if local.path.is_empty() => Err(
            ControllerError::InvalidSpec("local backend path must not be empty".to_string()),
        ),
        RepositoryBackend::S3(s3) if s3.bucket.is_empty() => Err(ControllerError::InvalidSpec(
            "s3 backend bucket must not be empty".to_string(),
        )),
        RepositoryBackend::S3(s3) if s3.credentials_secret_ref.is_empty() => Err(
            ControllerError::InvalidSpec("s3 credentialsSecretRef must not be empty".to_string()),
        ),
        _ => Ok(()),
    }
}

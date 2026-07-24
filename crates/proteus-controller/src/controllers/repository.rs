use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use k8s_openapi::api::core::v1::Secret;
use kube::api::{Patch, PatchParams};
use kube::runtime::controller::Action;
use kube::{Api, ResourceExt};
use proteus_core::{
    credentials_from_secret_data, LocalBackend, S3Backend, S3Config, S3Credentials,
};
use proteus_crd::{
    ProteusRepository, ProteusRepositoryStatus, RepositoryBackend, RepositoryPhase, S3BackendSpec,
};
use tracing::info;

use super::ReconcileCtx;
use crate::error::ControllerResult;

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

    let api: Api<ProteusRepository> = Api::namespaced(ctx.client.clone(), &ns);
    let checked_at = Utc::now().to_rfc3339();

    let outcome = probe_repository(&obj, &ctx, &ns).await;
    let status = match outcome {
        Ok(message) => ProteusRepositoryStatus {
            phase: Some(RepositoryPhase::Ready),
            message: Some(message),
            object_count: obj.status.as_ref().and_then(|s| s.object_count),
            bytes_stored: obj.status.as_ref().and_then(|s| s.bytes_stored),
            last_checked_at: Some(checked_at),
        },
        Err(message) => ProteusRepositoryStatus {
            phase: Some(RepositoryPhase::Failed),
            message: Some(message),
            object_count: obj.status.as_ref().and_then(|s| s.object_count),
            bytes_stored: obj.status.as_ref().and_then(|s| s.bytes_stored),
            last_checked_at: Some(checked_at),
        },
    };

    let patch = serde_json::json!({ "status": status });
    api.patch_status(&name, &PatchParams::default(), &Patch::Merge(&patch))
        .await?;

    if let Err(err) = ctx.api_state.refresh_counts().await {
        tracing::warn!(error = %err, "failed to refresh cluster snapshot counts");
        ctx.api_state.mark_reconciled();
    }

    let requeue = if matches!(status.phase.as_ref(), Some(RepositoryPhase::Failed)) {
        Duration::from_secs(60)
    } else {
        Duration::from_secs(300)
    };
    Ok(Action::requeue(requeue))
}

async fn probe_repository(
    obj: &ProteusRepository,
    ctx: &ReconcileCtx,
    namespace: &str,
) -> Result<String, String> {
    match &obj.spec.backend {
        RepositoryBackend::Local(local) => {
            if local.path.trim().is_empty() {
                return Err("local backend path must not be empty".to_string());
            }
            LocalBackend::probe(&local.path)
                .await
                .map_err(|err| format!("local path not writable: {err}"))?;
            Ok(format!("local repository ready at {}", local.path))
        }
        RepositoryBackend::S3(s3) => {
            validate_s3_fields(s3)?;
            let credentials =
                load_s3_credentials(ctx, namespace, &s3.credentials_secret_ref).await?;
            let backend = S3Backend::new(s3_config_from_spec(s3), credentials)
                .map_err(|err| format!("failed to build S3 client: {err}"))?;
            backend
                .probe()
                .await
                .map_err(|err| format!("S3 probe failed: {err}"))?;
            Ok(format!("S3 repository ready (bucket {})", s3.bucket))
        }
    }
}

fn validate_s3_fields(s3: &S3BackendSpec) -> Result<(), String> {
    if s3.bucket.trim().is_empty() {
        return Err("s3 backend bucket must not be empty".to_string());
    }
    if s3.credentials_secret_ref.trim().is_empty() {
        return Err("s3 credentialsSecretRef must not be empty".to_string());
    }
    Ok(())
}

fn s3_config_from_spec(s3: &S3BackendSpec) -> S3Config {
    S3Config {
        bucket: s3.bucket.clone(),
        prefix: s3.prefix.clone(),
        endpoint: s3.endpoint.clone(),
        region: s3.region.clone(),
        force_path_style: s3.force_path_style,
    }
}

async fn load_s3_credentials(
    ctx: &ReconcileCtx,
    namespace: &str,
    secret_name: &str,
) -> Result<S3Credentials, String> {
    let secrets: Api<Secret> = Api::namespaced(ctx.client.clone(), namespace);
    let secret = secrets
        .get(secret_name)
        .await
        .map_err(|err| format!("credentials Secret '{secret_name}' not found: {err}"))?;

    let decoded = decode_secret_string_data(secret.data.as_ref(), secret.string_data.as_ref());
    credentials_from_secret_data(&decoded).map_err(|err| err.to_string())
}

fn decode_secret_string_data(
    data: Option<&BTreeMap<String, k8s_openapi::ByteString>>,
    string_data: Option<&BTreeMap<String, String>>,
) -> std::collections::HashMap<String, String> {
    let mut out = std::collections::HashMap::new();
    if let Some(string_data) = string_data {
        for (k, v) in string_data {
            out.insert(k.clone(), v.clone());
        }
    }
    if let Some(data) = data {
        for (k, v) in data {
            if let Ok(s) = String::from_utf8(v.0.clone()) {
                out.entry(k.clone()).or_insert(s);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_prefers_string_data() {
        let mut string_data = BTreeMap::new();
        string_data.insert("accessKeyId".into(), "from-string".into());
        string_data.insert("secretAccessKey".into(), "sec".into());
        let decoded = decode_secret_string_data(None, Some(&string_data));
        assert_eq!(
            decoded.get("accessKeyId").map(String::as_str),
            Some("from-string")
        );
    }

    #[test]
    fn validate_s3_rejects_empty_bucket() {
        let s3 = S3BackendSpec {
            bucket: String::new(),
            prefix: None,
            endpoint: None,
            region: None,
            credentials_secret_ref: "creds".into(),
            force_path_style: false,
        };
        let err = validate_s3_fields(&s3).expect_err("empty bucket");
        assert!(err.contains("bucket"));
    }
}

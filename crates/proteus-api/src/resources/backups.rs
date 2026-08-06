use chrono::Utc;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
use kube::api::{DeleteParams, ListParams, PostParams};
use kube::{Api, ResourceExt};
use proteus_crd::{
    BackupPolicyPhase, ProteusBackup, ProteusBackupPolicy, ProteusBackupSpec, RetentionPolicy,
};
use serde::{Deserialize, Serialize};
use tracing::warn;

use super::common::{optional_trimmed, phase_label, require_non_empty};
use super::repo_store::gc_repository_after_backup_delete;
use crate::error::{ApiError, ApiResult};
use crate::state::{object_namespace, ApiState};

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupListItem {
    pub name: String,
    pub namespace: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub policy_ref: Option<String>,
    pub repository_ref: String,
    pub target_namespace: String,
    pub pvc_names: Vec<String>,
    pub schedule: Option<String>,
    pub phase: Option<String>,
    pub message: Option<String>,
    pub last_snapshot_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub progress_percent: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_seconds: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub throughput_bytes_per_sec: Option<u64>,
    /// When the run started (status.startedAt, RFC3339).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started_at: Option<String>,
    /// CR creation time (metadata.creationTimestamp, RFC3339).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateBackupRequest {
    /// Run name; optional when `policyRef` is set (auto-generated).
    #[serde(default)]
    pub name: Option<String>,
    /// Namespace for the `ProteusBackup` CR; defaults to policy namespace or `targetNamespace`.
    #[serde(default)]
    pub namespace: Option<String>,
    /// Preferred path: create a run from a policy ("Run now").
    #[serde(default)]
    pub policy_ref: Option<String>,
    #[serde(default)]
    pub policy_namespace: Option<String>,
    /// Legacy inline recipe (compat / tests). Ignored when `policyRef` is set.
    #[serde(default)]
    pub repository_ref: Option<String>,
    #[serde(default)]
    pub repository_namespace: Option<String>,
    #[serde(default)]
    pub target_namespace: Option<String>,
    #[serde(default)]
    pub pvc_names: Vec<String>,
}

fn backup_list_item(obj: &ProteusBackup) -> BackupListItem {
    BackupListItem {
        name: obj.name_any(),
        namespace: object_namespace(obj),
        policy_ref: obj.spec.policy_ref.clone(),
        repository_ref: obj.spec.repository_ref.clone(),
        target_namespace: obj.spec.target_namespace.clone(),
        pvc_names: obj.spec.pvc_names.clone(),
        schedule: obj.spec.schedule.clone(),
        phase: obj
            .status
            .as_ref()
            .and_then(|s| s.phase.as_ref().and_then(phase_label)),
        message: obj.status.as_ref().and_then(|s| s.message.clone()),
        last_snapshot_id: obj.status.as_ref().and_then(|s| s.last_snapshot_id.clone()),
        progress_percent: obj.status.as_ref().and_then(|s| s.progress_percent),
        duration_seconds: obj.status.as_ref().and_then(|s| s.duration_seconds),
        throughput_bytes_per_sec: obj.status.as_ref().and_then(|s| s.throughput_bytes_per_sec),
        started_at: obj.status.as_ref().and_then(|s| s.started_at.clone()),
        created_at: obj
            .metadata
            .creation_timestamp
            .as_ref()
            .map(|t| t.0.to_rfc3339()),
    }
}

pub async fn list_backups(state: &ApiState) -> ApiResult<Vec<BackupListItem>> {
    let api: Api<ProteusBackup> = Api::all(state.client.clone());
    let list = api.list(&ListParams::default()).await?;
    Ok(list.items.iter().map(backup_list_item).collect())
}

fn generate_run_name(policy_name: &str) -> String {
    proteus_core::backup_run_name(policy_name, Utc::now())
}

pub fn build_inline_backup(req: &CreateBackupRequest) -> ApiResult<(String, ProteusBackup)> {
    let name = require_non_empty("name", req.name.as_deref())?;
    let target_namespace = require_non_empty("targetNamespace", req.target_namespace.as_deref())?;
    let repository_ref = require_non_empty("repositoryRef", req.repository_ref.as_deref())?;
    let namespace =
        optional_trimmed(req.namespace.as_deref()).unwrap_or_else(|| target_namespace.clone());

    let pvc_names: Vec<String> = req
        .pvc_names
        .iter()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    if pvc_names.is_empty() {
        return Err(ApiError::BadRequest(
            "pvcNames must contain at least one PVC name".to_string(),
        ));
    }

    let backup = ProteusBackup {
        metadata: ObjectMeta {
            name: Some(name),
            namespace: Some(namespace.clone()),
            ..ObjectMeta::default()
        },
        spec: ProteusBackupSpec {
            policy_ref: None,
            policy_namespace: None,
            repository_ref,
            repository_namespace: optional_trimmed(req.repository_namespace.as_deref()),
            target_namespace,
            pvc_names,
            label_selector: None,
            schedule: None,
            retention: RetentionPolicy::default(),
            include_volumes: true,
            include_cluster_resources: false,
        },
        status: None,
    };
    Ok((namespace, backup))
}

fn build_run_from_policy(
    req: &CreateBackupRequest,
    policy: &ProteusBackupPolicy,
    policy_namespace: &str,
) -> ApiResult<(String, ProteusBackup)> {
    let policy_name = policy.name_any();
    let name = match optional_trimmed(req.name.as_deref()) {
        Some(n) => n,
        None => generate_run_name(&policy_name),
    };
    let namespace =
        optional_trimmed(req.namespace.as_deref()).unwrap_or_else(|| policy_namespace.to_string());

    let backup = ProteusBackup {
        metadata: ObjectMeta {
            name: Some(name),
            namespace: Some(namespace.clone()),
            ..ObjectMeta::default()
        },
        spec: ProteusBackupSpec {
            policy_ref: Some(policy_name),
            policy_namespace: Some(policy_namespace.to_string()),
            // Stamp recipe for list/GC; controller still resolves live policy via policyRef.
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
    Ok((namespace, backup))
}

pub async fn create_backup(
    state: &ApiState,
    req: CreateBackupRequest,
) -> ApiResult<BackupListItem> {
    let (namespace, backup) = if let Some(policy_ref) = optional_trimmed(req.policy_ref.as_deref())
    {
        let policy_namespace = optional_trimmed(req.policy_namespace.as_deref())
            .or_else(|| optional_trimmed(req.namespace.as_deref()))
            .ok_or_else(|| {
                ApiError::BadRequest(
                    "policyNamespace (or namespace) is required when policyRef is set".to_string(),
                )
            })?;
        let api: Api<ProteusBackupPolicy> =
            Api::namespaced(state.client.clone(), &policy_namespace);
        let policy = api.get(&policy_ref).await.map_err(|err| {
            ApiError::BadRequest(format!(
                "policyRef '{policy_ref}' in '{policy_namespace}': {err}"
            ))
        })?;
        match policy.status.as_ref().and_then(|s| s.phase.as_ref()) {
            Some(BackupPolicyPhase::Ready) => {}
            Some(BackupPolicyPhase::Invalid) => {
                let message = policy
                    .status
                    .as_ref()
                    .and_then(|s| s.message.as_deref())
                    .unwrap_or("policy is Invalid");
                return Err(ApiError::BadRequest(format!(
                    "policyRef '{policy_ref}' in '{policy_namespace}' is Invalid: {message}"
                )));
            }
            None => {
                return Err(ApiError::BadRequest(format!(
                    "policyRef '{policy_ref}' in '{policy_namespace}' is not Ready yet"
                )));
            }
        }
        build_run_from_policy(&req, &policy, &policy_namespace)?
    } else {
        build_inline_backup(&req)?
    };

    let api: Api<ProteusBackup> = Api::namespaced(state.client.clone(), &namespace);
    let created = api.create(&PostParams::default(), &backup).await?;
    let _ = state.refresh_counts().await;
    Ok(backup_list_item(&created))
}

/// Repository identity for GC: prefer live policy when `policyRef` is set, else stamped/inline.
pub(crate) async fn resolve_backup_repository(
    state: &ApiState,
    backup: &ProteusBackup,
    backup_namespace: &str,
) -> Result<(String, String), String> {
    if let Some(policy_ref) = backup
        .spec
        .policy_ref
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        let policy_ns = backup
            .spec
            .policy_namespace
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or(backup_namespace);
        let api: Api<ProteusBackupPolicy> = Api::namespaced(state.client.clone(), policy_ns);
        match api.get(policy_ref).await {
            Ok(policy) => {
                let repo_ns = policy
                    .spec
                    .repository_namespace
                    .as_deref()
                    .unwrap_or(policy_ns)
                    .to_string();
                if policy.spec.repository_ref.trim().is_empty() {
                    return Err(format!(
                        "policy '{policy_ref}' in '{policy_ns}' has empty repositoryRef"
                    ));
                }
                return Ok((repo_ns, policy.spec.repository_ref.clone()));
            }
            Err(err) => {
                if !backup.spec.repository_ref.trim().is_empty() {
                    let repo_ns = backup
                        .spec
                        .repository_namespace
                        .as_deref()
                        .unwrap_or(backup_namespace)
                        .to_string();
                    return Ok((repo_ns, backup.spec.repository_ref.clone()));
                }
                return Err(format!(
                    "failed to load policy '{policy_ref}' for repository GC: {err}"
                ));
            }
        }
    }

    if backup.spec.repository_ref.trim().is_empty() {
        return Err(
            "backup has neither repositoryRef nor policyRef; cannot purge repository".into(),
        );
    }
    let repo_ns = backup
        .spec
        .repository_namespace
        .as_deref()
        .unwrap_or(backup_namespace)
        .to_string();
    Ok((repo_ns, backup.spec.repository_ref.clone()))
}

pub async fn delete_backup(state: &ApiState, namespace: &str, name: &str) -> ApiResult<()> {
    let api: Api<ProteusBackup> = Api::namespaced(state.client.clone(), namespace);
    let backup = api.get(name).await?;

    let (repo_ns, repo_ref) = resolve_backup_repository(state, &backup, namespace)
        .await
        .map_err(ApiError::Internal)?;

    // GC first: drop this backup's snapshot + orphans; keep other backups' objects.
    match gc_repository_after_backup_delete(state, namespace, name, &repo_ns, &repo_ref).await {
        Ok(removed) => {
            if removed > 0 {
                tracing::info!(
                    backup = %name,
                    repository = %repo_ref,
                    removed,
                    "purged unreferenced objects from repository"
                );
            }
        }
        Err(err) => {
            warn!(
                backup = %name,
                error = %err,
                "failed to purge repository objects; leaving CR in place"
            );
            return Err(ApiError::Internal(format!(
                "could not remove backup data from repository: {err}"
            )));
        }
    }

    api.delete(name, &DeleteParams::default()).await?;
    let _ = state.refresh_counts().await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use proteus_crd::ProteusBackupPolicySpec;

    #[test]
    fn build_inline_rejects_empty_pvc_names() {
        let req = CreateBackupRequest {
            name: Some("b1".into()),
            namespace: None,
            policy_ref: None,
            policy_namespace: None,
            repository_ref: Some("repo-1".into()),
            repository_namespace: None,
            target_namespace: Some("workloads".into()),
            pvc_names: vec![],
        };
        let err = build_inline_backup(&req).expect_err("pvcNames required");
        assert!(err.to_string().contains("pvcNames"));
    }

    #[test]
    fn build_inline_defaults_namespace_to_target_namespace() {
        let req = CreateBackupRequest {
            name: Some("b1".into()),
            namespace: None,
            policy_ref: None,
            policy_namespace: None,
            repository_ref: Some("repo-1".into()),
            repository_namespace: None,
            target_namespace: Some("workloads".into()),
            pvc_names: vec!["data-pvc".into()],
        };
        let (namespace, backup) = build_inline_backup(&req).expect("valid");
        assert_eq!(namespace, "workloads");
        assert_eq!(backup.metadata.namespace.as_deref(), Some("workloads"));
        assert_eq!(backup.spec.pvc_names, vec!["data-pvc".to_string()]);
    }

    #[test]
    fn build_run_from_policy_stamps_recipe_and_policy_ref() {
        let policy = ProteusBackupPolicy::new(
            "nightly",
            ProteusBackupPolicySpec {
                repository_ref: "repo".into(),
                repository_namespace: Some("proteus-system".into()),
                target_namespace: "workloads".into(),
                pvc_names: vec!["data".into()],
                label_selector: None,
                schedule: None,
                paused: false,
                retention: RetentionPolicy {
                    keep_last: 3,
                    max_age_days: None,
                },
                include_volumes: true,
                include_cluster_resources: false,
            },
        );
        let req = CreateBackupRequest {
            name: Some("nightly-1".into()),
            namespace: None,
            policy_ref: Some("nightly".into()),
            policy_namespace: Some("proteus-system".into()),
            repository_ref: None,
            repository_namespace: None,
            target_namespace: None,
            pvc_names: vec![],
        };
        let (ns, backup) = build_run_from_policy(&req, &policy, "proteus-system").expect("valid");
        assert_eq!(ns, "proteus-system");
        assert_eq!(backup.spec.policy_ref.as_deref(), Some("nightly"));
        assert_eq!(backup.spec.repository_ref, "repo");
        assert_eq!(backup.spec.pvc_names, vec!["data".to_string()]);
    }

    #[test]
    fn generate_run_name_includes_policy() {
        let name = generate_run_name("nightly");
        assert!(name.starts_with("nightly-"));
        assert!(name.len() <= 63);
    }
}

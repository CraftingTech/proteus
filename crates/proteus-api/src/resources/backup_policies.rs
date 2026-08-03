use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
use kube::api::{DeleteParams, ListParams, PostParams};
use kube::{Api, ResourceExt};
use proteus_crd::{ProteusBackupPolicy, ProteusBackupPolicySpec, RetentionPolicy};
use serde::{Deserialize, Serialize};

use super::common::{optional_trimmed, phase_label, require_non_empty, resolve_namespace};
use crate::error::{ApiError, ApiResult};
use crate::state::{object_namespace, ApiState};

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupPolicyListItem {
    pub name: String,
    pub namespace: String,
    pub repository_ref: String,
    pub repository_namespace: Option<String>,
    pub target_namespace: String,
    pub pvc_names: Vec<String>,
    pub schedule: Option<String>,
    pub keep_last: u32,
    pub max_age_days: Option<u32>,
    pub phase: Option<String>,
    pub message: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateBackupPolicyRequest {
    pub name: String,
    #[serde(default)]
    pub namespace: Option<String>,
    pub repository_ref: String,
    #[serde(default)]
    pub repository_namespace: Option<String>,
    pub target_namespace: String,
    #[serde(default)]
    pub pvc_names: Vec<String>,
    #[serde(default)]
    pub schedule: Option<String>,
    #[serde(default)]
    pub keep_last: Option<u32>,
    #[serde(default)]
    pub max_age_days: Option<u32>,
}

fn policy_list_item(obj: &ProteusBackupPolicy) -> BackupPolicyListItem {
    BackupPolicyListItem {
        name: obj.name_any(),
        namespace: object_namespace(obj),
        repository_ref: obj.spec.repository_ref.clone(),
        repository_namespace: obj.spec.repository_namespace.clone(),
        target_namespace: obj.spec.target_namespace.clone(),
        pvc_names: obj.spec.pvc_names.clone(),
        schedule: obj.spec.schedule.clone(),
        keep_last: obj.spec.retention.keep_last,
        max_age_days: obj.spec.retention.max_age_days,
        phase: obj
            .status
            .as_ref()
            .and_then(|s| s.phase.as_ref().and_then(phase_label)),
        message: obj.status.as_ref().and_then(|s| s.message.clone()),
    }
}

pub async fn list_backup_policies(state: &ApiState) -> ApiResult<Vec<BackupPolicyListItem>> {
    let api: Api<ProteusBackupPolicy> = Api::all(state.client.clone());
    let list = api.list(&ListParams::default()).await?;
    Ok(list.items.iter().map(policy_list_item).collect())
}

pub fn build_backup_policy(
    req: &CreateBackupPolicyRequest,
) -> ApiResult<(String, ProteusBackupPolicy)> {
    let name = require_non_empty("name", Some(req.name.as_str()))?;
    let target_namespace =
        require_non_empty("targetNamespace", Some(req.target_namespace.as_str()))?;
    let repository_ref = require_non_empty("repositoryRef", Some(req.repository_ref.as_str()))?;
    let namespace = match optional_trimmed(req.namespace.as_deref()) {
        Some(ns) => ns,
        None => resolve_namespace(None)?,
    };

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

    let keep_last = req.keep_last.unwrap_or(7);
    if keep_last == 0 {
        return Err(ApiError::BadRequest("keepLast must be >= 1".to_string()));
    }

    let policy = ProteusBackupPolicy {
        metadata: ObjectMeta {
            name: Some(name),
            namespace: Some(namespace.clone()),
            ..ObjectMeta::default()
        },
        spec: ProteusBackupPolicySpec {
            repository_ref,
            repository_namespace: optional_trimmed(req.repository_namespace.as_deref()),
            target_namespace,
            pvc_names,
            label_selector: None,
            schedule: optional_trimmed(req.schedule.as_deref()),
            retention: RetentionPolicy {
                keep_last,
                max_age_days: req.max_age_days,
            },
            include_volumes: true,
            include_cluster_resources: false,
        },
        status: None,
    };
    Ok((namespace, policy))
}

pub async fn create_backup_policy(
    state: &ApiState,
    req: CreateBackupPolicyRequest,
) -> ApiResult<BackupPolicyListItem> {
    let (namespace, policy) = build_backup_policy(&req)?;
    let api: Api<ProteusBackupPolicy> = Api::namespaced(state.client.clone(), &namespace);
    let created = api.create(&PostParams::default(), &policy).await?;
    let _ = state.refresh_counts().await;
    Ok(policy_list_item(&created))
}

pub async fn delete_backup_policy(state: &ApiState, namespace: &str, name: &str) -> ApiResult<()> {
    let api: Api<ProteusBackupPolicy> = Api::namespaced(state.client.clone(), namespace);
    api.delete(name, &DeleteParams::default()).await?;
    let _ = state.refresh_counts().await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_policy_rejects_empty_pvcs() {
        let req = CreateBackupPolicyRequest {
            name: "nightly".into(),
            namespace: Some("proteus-system".into()),
            repository_ref: "repo".into(),
            repository_namespace: None,
            target_namespace: "workloads".into(),
            pvc_names: vec![],
            schedule: None,
            keep_last: None,
            max_age_days: None,
        };
        assert!(build_backup_policy(&req).is_err());
    }

    #[test]
    fn build_policy_defaults_namespace() {
        let req = CreateBackupPolicyRequest {
            name: "nightly".into(),
            namespace: None,
            repository_ref: "repo".into(),
            repository_namespace: None,
            target_namespace: "workloads".into(),
            pvc_names: vec!["data".into()],
            schedule: None,
            keep_last: Some(3),
            max_age_days: None,
        };
        let (ns, policy) = build_backup_policy(&req).expect("valid");
        assert_eq!(ns, "proteus-system");
        assert_eq!(policy.spec.retention.keep_last, 3);
        assert_eq!(policy.spec.pvc_names, vec!["data".to_string()]);
    }
}

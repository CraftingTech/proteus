use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
use kube::api::{DeleteParams, ListParams, Patch, PatchParams, PostParams};
use kube::{Api, ResourceExt};
use proteus_core::validate_schedule;
use proteus_crd::{ProteusBackupPolicy, ProteusBackupPolicySpec, RetentionPolicy};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

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
    pub paused: bool,
    pub keep_last: u32,
    pub max_age_days: Option<u32>,
    pub phase: Option<String>,
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_run_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_schedule_time: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_run_name: Option<String>,
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
    pub paused: Option<bool>,
    #[serde(default)]
    pub keep_last: Option<u32>,
    #[serde(default)]
    pub max_age_days: Option<u32>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PatchBackupPolicyRequest {
    /// Set cron; empty string clears the schedule.
    #[serde(default)]
    pub schedule: Option<String>,
    #[serde(default)]
    pub paused: Option<bool>,
    #[serde(default)]
    pub keep_last: Option<u32>,
    #[serde(default)]
    pub max_age_days: Option<u32>,
    /// When true, clears `maxAgeDays` (merge patch cannot express null via Option alone).
    #[serde(default)]
    pub clear_max_age_days: Option<bool>,
    #[serde(default)]
    pub pvc_names: Option<Vec<String>>,
}

fn policy_list_item(obj: &ProteusBackupPolicy) -> BackupPolicyListItem {
    let status = obj.status.as_ref();
    BackupPolicyListItem {
        name: obj.name_any(),
        namespace: object_namespace(obj),
        repository_ref: obj.spec.repository_ref.clone(),
        repository_namespace: obj.spec.repository_namespace.clone(),
        target_namespace: obj.spec.target_namespace.clone(),
        pvc_names: obj.spec.pvc_names.clone(),
        schedule: obj.spec.schedule.clone(),
        paused: obj.spec.paused,
        keep_last: obj.spec.retention.keep_last,
        max_age_days: obj.spec.retention.max_age_days,
        phase: status.and_then(|s| s.phase.as_ref().and_then(phase_label)),
        message: status.and_then(|s| s.message.clone()),
        next_run_at: status.and_then(|s| s.next_run_at.clone()),
        last_schedule_time: status.and_then(|s| s.last_schedule_time.clone()),
        last_run_name: status.and_then(|s| s.last_run_name.clone()),
    }
}

fn validate_optional_schedule(schedule: Option<&str>) -> ApiResult<Option<String>> {
    let Some(raw) = optional_trimmed(schedule) else {
        return Ok(None);
    };
    validate_schedule(&raw).map_err(ApiError::BadRequest)?;
    Ok(Some(raw))
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

    let schedule = validate_optional_schedule(req.schedule.as_deref())?;

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
            schedule,
            paused: req.paused.unwrap_or(false),
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

pub async fn list_backup_policies(state: &ApiState) -> ApiResult<Vec<BackupPolicyListItem>> {
    let api: Api<ProteusBackupPolicy> = Api::all(state.client.clone());
    let list = api.list(&ListParams::default()).await?;
    Ok(list.items.iter().map(policy_list_item).collect())
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

/// Merge selected policy fields. Empty `schedule` clears the cron.
pub fn build_policy_patch(req: &PatchBackupPolicyRequest) -> ApiResult<Value> {
    if req.schedule.is_none()
        && req.paused.is_none()
        && req.keep_last.is_none()
        && req.max_age_days.is_none()
        && req.clear_max_age_days.is_none()
        && req.pvc_names.is_none()
    {
        return Err(ApiError::BadRequest(
            "at least one of schedule, paused, keepLast, maxAgeDays, or pvcNames is required"
                .to_string(),
        ));
    }

    let mut spec = Map::new();

    if let Some(schedule) = &req.schedule {
        let validated = validate_optional_schedule(Some(schedule.as_str()))?;
        match validated {
            None => {
                spec.insert("schedule".to_string(), Value::Null);
            }
            Some(s) => {
                spec.insert("schedule".to_string(), Value::String(s));
            }
        }
    }

    if let Some(paused) = req.paused {
        spec.insert("paused".to_string(), Value::Bool(paused));
    }

    let touch_retention = req.keep_last.is_some()
        || req.max_age_days.is_some()
        || req.clear_max_age_days == Some(true);
    if touch_retention {
        let mut retention = Map::new();
        if let Some(keep_last) = req.keep_last {
            if keep_last == 0 {
                return Err(ApiError::BadRequest("keepLast must be >= 1".to_string()));
            }
            retention.insert("keepLast".to_string(), Value::from(keep_last));
        }
        if req.clear_max_age_days == Some(true) {
            retention.insert("maxAgeDays".to_string(), Value::Null);
        } else if let Some(days) = req.max_age_days {
            retention.insert("maxAgeDays".to_string(), Value::from(days));
        }
        spec.insert("retention".to_string(), Value::Object(retention));
    }

    if let Some(pvcs) = &req.pvc_names {
        let pvc_names: Vec<String> = pvcs
            .iter()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        if pvc_names.is_empty() {
            return Err(ApiError::BadRequest(
                "pvcNames must contain at least one PVC name".to_string(),
            ));
        }
        spec.insert(
            "pvcNames".to_string(),
            Value::Array(pvc_names.into_iter().map(Value::String).collect()),
        );
    }

    let mut patch = Map::new();
    patch.insert("spec".to_string(), Value::Object(spec));
    Ok(Value::Object(patch))
}

pub async fn patch_backup_policy(
    state: &ApiState,
    namespace: &str,
    name: &str,
    req: PatchBackupPolicyRequest,
) -> ApiResult<BackupPolicyListItem> {
    let body = build_policy_patch(&req)?;
    let api: Api<ProteusBackupPolicy> = Api::namespaced(state.client.clone(), namespace);
    let updated = api
        .patch(name, &PatchParams::default(), &Patch::Merge(&body))
        .await?;
    let _ = state.refresh_counts().await;
    Ok(policy_list_item(&updated))
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
            paused: None,
            keep_last: None,
            max_age_days: None,
        };
        assert!(build_backup_policy(&req).is_err());
    }

    #[test]
    fn build_policy_defaults_namespace_and_paused() {
        let req = CreateBackupPolicyRequest {
            name: "nightly".into(),
            namespace: None,
            repository_ref: "repo".into(),
            repository_namespace: None,
            target_namespace: "workloads".into(),
            pvc_names: vec!["data".into()],
            schedule: Some("0 2 * * *".into()),
            paused: None,
            keep_last: Some(3),
            max_age_days: None,
        };
        let (ns, policy) = build_backup_policy(&req).expect("valid");
        assert_eq!(ns, "proteus-system");
        assert_eq!(policy.spec.retention.keep_last, 3);
        assert_eq!(policy.spec.pvc_names, vec!["data".to_string()]);
        assert!(!policy.spec.paused);
        assert_eq!(policy.spec.schedule.as_deref(), Some("0 2 * * *"));
    }

    #[test]
    fn build_policy_rejects_bad_cron() {
        let req = CreateBackupPolicyRequest {
            name: "nightly".into(),
            namespace: Some("ns".into()),
            repository_ref: "repo".into(),
            repository_namespace: None,
            target_namespace: "workloads".into(),
            pvc_names: vec!["data".into()],
            schedule: Some("garbage".into()),
            paused: None,
            keep_last: None,
            max_age_days: None,
        };
        assert!(build_backup_policy(&req).is_err());
    }

    #[test]
    fn patch_paused_true() {
        let body = build_policy_patch(&PatchBackupPolicyRequest {
            schedule: None,
            paused: Some(true),
            keep_last: None,
            max_age_days: None,
            clear_max_age_days: None,
            pvc_names: None,
        })
        .expect("ok");
        assert_eq!(body["spec"]["paused"], Value::Bool(true));
    }

    #[test]
    fn patch_rejects_bad_cron() {
        let err = build_policy_patch(&PatchBackupPolicyRequest {
            schedule: Some("nope".into()),
            paused: None,
            keep_last: None,
            max_age_days: None,
            clear_max_age_days: None,
            pvc_names: None,
        });
        assert!(err.is_err());
    }

    #[test]
    fn patch_clear_schedule() {
        let body = build_policy_patch(&PatchBackupPolicyRequest {
            schedule: Some(String::new()),
            paused: None,
            keep_last: None,
            max_age_days: None,
            clear_max_age_days: None,
            pvc_names: None,
        })
        .expect("ok");
        assert_eq!(body["spec"]["schedule"], Value::Null);
    }
}

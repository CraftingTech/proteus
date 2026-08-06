use kube::CustomResource;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::backup::RetentionPolicy;

fn default_true() -> bool {
    true
}

#[derive(CustomResource, Deserialize, Serialize, Clone, Debug, JsonSchema)]
#[kube(
    group = "proteus.io",
    version = "v1alpha1",
    kind = "ProteusBackupPolicy",
    plural = "proteusbackuppolicies",
    shortname = "pbackuppolicy",
    status = "ProteusBackupPolicyStatus",
    namespaced,
    printcolumn = r#"{"name":"Repository","type":"string","jsonPath":".spec.repositoryRef"}"#,
    printcolumn = r#"{"name":"Schedule","type":"string","jsonPath":".spec.schedule"}"#,
    printcolumn = r#"{"name":"Phase","type":"string","jsonPath":".status.phase"}"#,
    printcolumn = r#"{"name":"Next","type":"string","jsonPath":".status.nextRunAt"}"#,
    printcolumn = r#"{"name":"Age","type":"date","jsonPath":".metadata.creationTimestamp"}"#
)]
#[serde(rename_all = "camelCase")]
pub struct ProteusBackupPolicySpec {
    pub repository_ref: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repository_namespace: Option<String>,
    pub target_namespace: String,
    /// PVCs to back up, by name, in `target_namespace`. Required, at least one.
    #[schemars(length(min = 1))]
    pub pvc_names: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label_selector: Option<String>,
    /// 5-field crontab (`min hour dom month dow`) or 6-field with seconds; UTC.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schedule: Option<String>,
    /// When true, scheduled ticks do not create runs (manual Run now still allowed).
    #[serde(default)]
    pub paused: bool,
    #[serde(default)]
    pub retention: RetentionPolicy,
    #[serde(default = "default_true")]
    pub include_volumes: bool,
    #[serde(default)]
    pub include_cluster_resources: bool,
}

#[derive(Deserialize, Serialize, Clone, Debug, JsonSchema, Default)]
#[serde(rename_all = "camelCase")]
pub struct ProteusBackupPolicyStatus {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase: Option<BackupPolicyPhase>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// Next scheduled fire time (RFC3339 UTC), when a schedule is set and not paused.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_run_at: Option<String>,
    /// When the controller last spawned a scheduled run (RFC3339 UTC).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_schedule_time: Option<String>,
    /// Name of the last scheduled `ProteusBackup` run created by the controller.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_run_name: Option<String>,
}

#[derive(Deserialize, Serialize, Clone, Debug, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "PascalCase")]
pub enum BackupPolicyPhase {
    Ready,
    Invalid,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deserializes_policy_recipe() {
        let yaml = r#"
apiVersion: proteus.io/v1alpha1
kind: ProteusBackupPolicy
metadata:
  name: nightly
  namespace: default
spec:
  repositoryRef: local-repo
  targetNamespace: demo
  pvcNames:
    - demo-data
  schedule: "0 2 * * *"
  paused: true
  retention:
    keepLast: 3
"#;
        let policy: ProteusBackupPolicy = serde_yaml::from_str(yaml).expect("policy deserializes");
        assert_eq!(policy.spec.repository_ref, "local-repo");
        assert_eq!(policy.spec.schedule.as_deref(), Some("0 2 * * *"));
        assert!(policy.spec.paused);
        assert_eq!(policy.spec.retention.keep_last, 3);
    }

    #[test]
    fn paused_defaults_false() {
        let yaml = r#"
apiVersion: proteus.io/v1alpha1
kind: ProteusBackupPolicy
metadata:
  name: nightly
  namespace: default
spec:
  repositoryRef: local-repo
  targetNamespace: demo
  pvcNames:
    - demo-data
"#;
        let policy: ProteusBackupPolicy = serde_yaml::from_str(yaml).expect("policy deserializes");
        assert!(!policy.spec.paused);
        assert!(policy.status.is_none() || policy.status.as_ref().unwrap().next_run_at.is_none());
    }
}

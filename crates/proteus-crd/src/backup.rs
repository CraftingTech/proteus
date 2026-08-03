use kube::CustomResource;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

fn default_true() -> bool {
    true
}

/// One backup execution (run). Recipe fields may come from `policyRef` or inline (legacy).
#[derive(CustomResource, Deserialize, Serialize, Clone, Debug, JsonSchema)]
#[kube(
    group = "proteus.io",
    version = "v1alpha1",
    kind = "ProteusBackup",
    plural = "proteusbackups",
    shortname = "pbackup",
    status = "ProteusBackupStatus",
    namespaced,
    printcolumn = r#"{"name":"Policy","type":"string","jsonPath":".spec.policyRef"}"#,
    printcolumn = r#"{"name":"Repository","type":"string","jsonPath":".spec.repositoryRef"}"#,
    printcolumn = r#"{"name":"Phase","type":"string","jsonPath":".status.phase"}"#,
    printcolumn = r#"{"name":"Age","type":"date","jsonPath":".metadata.creationTimestamp"}"#
)]
#[serde(rename_all = "camelCase")]
pub struct ProteusBackupSpec {
    /// When set, the controller loads the recipe from this `ProteusBackupPolicy`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy_namespace: Option<String>,
    /// Inline recipe (legacy / compat). Ignored when `policyRef` is set.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub repository_ref: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repository_namespace: Option<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub target_namespace: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pvc_names: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label_selector: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schedule: Option<String>,
    #[serde(default)]
    pub retention: RetentionPolicy,
    #[serde(default = "default_true")]
    pub include_volumes: bool,
    #[serde(default)]
    pub include_cluster_resources: bool,
}

#[derive(Deserialize, Serialize, Clone, Debug, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RetentionPolicy {
    #[serde(default = "default_keep_last")]
    pub keep_last: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_age_days: Option<u32>,
}

fn default_keep_last() -> u32 {
    7
}

impl Default for RetentionPolicy {
    fn default() -> Self {
        Self {
            keep_last: default_keep_last(),
            max_age_days: None,
        }
    }
}

#[derive(Deserialize, Serialize, Clone, Debug, JsonSchema, Default)]
#[serde(rename_all = "camelCase")]
pub struct ProteusBackupStatus {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase: Option<BackupPhase>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_snapshot_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_success_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_failure_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retained_snapshots: Option<u32>,
    /// 0–100 while `phase` is Running; 100 on Succeeded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub progress_percent: Option<u8>,
    /// When the current/last run started (RFC3339).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<String>,
    /// Wall-clock duration of a successful run, in seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_seconds: Option<u64>,
    /// Approximate throughput of a successful run (`lastBytes / durationSeconds`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub throughput_bytes_per_sec: Option<u64>,
}

#[derive(Deserialize, Serialize, Clone, Debug, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "PascalCase")]
pub enum BackupPhase {
    Pending,
    Running,
    Succeeded,
    Failed,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deserializes_legacy_inline_backup() {
        let yaml = r#"
apiVersion: proteus.io/v1alpha1
kind: ProteusBackup
metadata:
  name: demo
  namespace: default
spec:
  repositoryRef: local-repo
  targetNamespace: demo
  pvcNames:
    - demo-data
"#;
        let backup: ProteusBackup =
            serde_yaml::from_str(yaml).expect("legacy backup deserializes");
        assert!(backup.spec.policy_ref.is_none());
        assert_eq!(backup.spec.repository_ref, "local-repo");
        assert_eq!(backup.spec.pvc_names, vec!["demo-data".to_string()]);
    }

    #[test]
    fn deserializes_policy_ref_only_backup() {
        let yaml = r#"
apiVersion: proteus.io/v1alpha1
kind: ProteusBackup
metadata:
  name: demo-run
  namespace: default
spec:
  policyRef: nightly
"#;
        let backup: ProteusBackup =
            serde_yaml::from_str(yaml).expect("policyRef backup deserializes");
        assert_eq!(backup.spec.policy_ref.as_deref(), Some("nightly"));
        assert!(backup.spec.repository_ref.is_empty());
        assert!(backup.spec.pvc_names.is_empty());
    }
}

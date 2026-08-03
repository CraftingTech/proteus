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
    printcolumn = r#"{"name":"Phase","type":"string","jsonPath":".status.phase"}"#,
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
    /// Cron expression; unused until scheduled runs (#16).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schedule: Option<String>,
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
  retention:
    keepLast: 3
"#;
        let policy: ProteusBackupPolicy = serde_yaml::from_str(yaml).expect("policy deserializes");
        assert_eq!(policy.spec.repository_ref, "local-repo");
        assert_eq!(policy.spec.pvc_names, vec!["demo-data".to_string()]);
        assert_eq!(policy.spec.retention.keep_last, 3);
    }
}

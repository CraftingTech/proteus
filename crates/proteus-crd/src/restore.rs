use kube::CustomResource;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(CustomResource, Deserialize, Serialize, Clone, Debug, JsonSchema)]
#[kube(
    group = "proteus.io",
    version = "v1alpha1",
    kind = "ProteusRestore",
    plural = "proteusrestores",
    shortname = "prestore",
    status = "ProteusRestoreStatus",
    namespaced,
    printcolumn = r#"{"name":"Backup","type":"string","jsonPath":".spec.backupRef"}"#,
    printcolumn = r#"{"name":"TargetNS","type":"string","jsonPath":".spec.targetNamespace"}"#,
    printcolumn = r#"{"name":"Phase","type":"string","jsonPath":".status.phase"}"#,
    printcolumn = r#"{"name":"Age","type":"date","jsonPath":".metadata.creationTimestamp"}"#
)]
#[serde(rename_all = "camelCase")]
pub struct ProteusRestoreSpec {
    pub backup_ref: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backup_namespace: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snapshot_id: Option<String>,
    pub target_namespace: String,
    #[serde(default)]
    pub overwrite: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include_resources: Option<String>,
}

#[derive(Deserialize, Serialize, Clone, Debug, JsonSchema, Default)]
#[serde(rename_all = "camelCase")]
pub struct ProteusRestoreStatus {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase: Option<RestorePhase>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub restored_snapshot_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub progress_percent: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<String>,
    /// When the current/last run started (RFC3339).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<String>,
    /// Bytes written across restored volumes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_bytes: Option<u64>,
    /// Wall-clock duration of a successful run, in seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_seconds: Option<u64>,
    /// Approximate throughput (`lastBytes / durationSeconds`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub throughput_bytes_per_sec: Option<u64>,
    /// Bulk I/O path used for this run (`exec` | `agent`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data_plane: Option<crate::DataPlane>,
    /// Node that should run (or ran) an agent-plane restore.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assigned_node: Option<String>,
}

#[derive(Deserialize, Serialize, Clone, Debug, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "PascalCase")]
pub enum RestorePhase {
    Pending,
    Running,
    Succeeded,
    Failed,
}

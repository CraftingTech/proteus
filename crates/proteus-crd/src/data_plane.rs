use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Which bulk I/O path executed a Backup or Restore (ADR 0001).
#[derive(Deserialize, Serialize, Clone, Debug, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum DataPlane {
    /// Mount Pod + kube-exec into the controller (fallback / `just run`).
    Exec,
    /// Node-agent + mover Pod on the PVC's node.
    Agent,
}

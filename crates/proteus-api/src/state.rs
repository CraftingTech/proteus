use std::sync::Arc;

use parking_lot::RwLock;
use serde::Serialize;

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClusterSnapshot {
    pub version: String,
    pub repositories: u64,
    pub backups: u64,
    pub restores: u64,
    pub last_reconcile_at: Option<String>,
}

#[derive(Clone)]
pub struct ApiState {
    pub snapshot: Arc<RwLock<ClusterSnapshot>>,
}

impl ApiState {
    pub fn new(snapshot: ClusterSnapshot) -> Self {
        Self {
            snapshot: Arc::new(RwLock::new(snapshot)),
        }
    }
}

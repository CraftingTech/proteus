use serde::{Deserialize, Serialize};

/// One PVC's contribution to a snapshot: its chunk ids, in order, and total plaintext size.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VolumeSnapshot {
    pub pvc_name: String,
    pub bytes: u64,
    /// Hex-encoded `ContentId` per chunk, hashed over plaintext, in stream order.
    pub chunk_ids: Vec<String>,
}

/// Versioned manifest for one backup run, stored itself as a CAS object.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotManifest {
    pub version: u32,
    pub created_at: String,
    pub encrypted: bool,
    pub volumes: Vec<VolumeSnapshot>,
    pub total_bytes: u64,
}

pub const MANIFEST_VERSION: u32 = 1;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_round_trips_through_json() {
        let manifest = SnapshotManifest {
            version: MANIFEST_VERSION,
            created_at: "2026-07-24T00:00:00Z".to_string(),
            encrypted: true,
            volumes: vec![VolumeSnapshot {
                pvc_name: "data".to_string(),
                bytes: 42,
                chunk_ids: vec!["ab".repeat(32)],
            }],
            total_bytes: 42,
        };

        let json = serde_json::to_vec(&manifest).expect("serialize");
        let parsed: SnapshotManifest = serde_json::from_slice(&json).expect("deserialize");

        assert_eq!(parsed.version, manifest.version);
        assert_eq!(parsed.total_bytes, manifest.total_bytes);
        assert_eq!(parsed.volumes.len(), 1);
        assert_eq!(parsed.volumes[0].pvc_name, "data");
    }
}

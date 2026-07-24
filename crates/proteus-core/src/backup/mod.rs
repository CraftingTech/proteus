//! Snapshot pipeline: chunk plaintext, hash, optionally encrypt, put into a CAS `ObjectStore`.

mod pipeline;
mod snapshot;

pub use pipeline::{create_snapshot, ingest_volume_backup, seal_snapshot, SnapshotInput};
pub use snapshot::{SnapshotManifest, VolumeSnapshot, MANIFEST_VERSION};

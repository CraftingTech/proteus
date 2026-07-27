//! Snapshot pipeline: chunk plaintext, hash, optionally encrypt, put into a CAS `ObjectStore`.

mod pipeline;
mod snapshot;

pub use pipeline::{
    create_snapshot, create_snapshot_from_streams, create_snapshot_with_progress, gc_unreferenced,
    ingest_volume_backup, ingest_volume_backup_with_progress, ingest_volume_stream, load_snapshot,
    materialize_volume, seal_snapshot, SnapshotInput,
};
pub use snapshot::{SnapshotManifest, VolumeSnapshot, MANIFEST_VERSION};

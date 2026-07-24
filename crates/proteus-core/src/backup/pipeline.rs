use bytes::Bytes;

use super::snapshot::{SnapshotManifest, VolumeSnapshot, MANIFEST_VERSION};
use crate::chunking::Chunker;
use crate::crypto::{encrypt, EncryptionKey};
use crate::error::{CoreError, CoreResult};
use crate::hash::{hash_bytes, ContentId};
use crate::storage::{ObjectStore, PutOptions};

/// One volume's raw plaintext bytes, ready to chunk and store.
pub struct SnapshotInput<'a> {
    pub pvc_name: &'a str,
    pub data: &'a [u8],
}

/// Chunk `data`, hash each chunk over plaintext, optionally encrypt, then `put` into `store`.
///
/// The chunk id is always the plaintext hash (dedup key + restore lookup); when `key` is set the
/// stored blob is the AES-GCM ciphertext under that same id, so blobs at rest are never plaintext.
pub async fn ingest_volume_backup(
    store: &dyn ObjectStore,
    key: Option<&EncryptionKey>,
    pvc_name: &str,
    data: &[u8],
) -> CoreResult<VolumeSnapshot> {
    let chunker = Chunker::default();
    let chunks = chunker.chunk(data);

    let mut chunk_ids = Vec::with_capacity(chunks.len());
    let mut bytes = 0u64;

    for chunk in chunks {
        bytes += chunk.data.len() as u64;
        let payload = match key {
            Some(key) => Bytes::from(encrypt(key, &chunk.data)?.blob),
            None => chunk.data,
        };
        store.put(&chunk.id, payload, PutOptions::default()).await?;
        chunk_ids.push(chunk.id.to_hex());
    }

    Ok(VolumeSnapshot {
        pvc_name: pvc_name.to_string(),
        bytes,
        chunk_ids,
    })
}

/// Serialize `manifest` and `put` it into `store`, keyed by its own content hash.
pub async fn seal_snapshot(
    store: &dyn ObjectStore,
    manifest: &SnapshotManifest,
) -> CoreResult<ContentId> {
    let json = serde_json::to_vec(manifest)
        .map_err(|err| CoreError::InvalidArgument(format!("manifest serialize: {err}")))?;
    let id = hash_bytes(&json);
    store
        .put(&id, Bytes::from(json), PutOptions::default())
        .await?;
    Ok(id)
}

/// Full pipeline entry point: ingest every volume, seal the manifest, return its id + total bytes.
pub async fn create_snapshot(
    store: &dyn ObjectStore,
    key: Option<&EncryptionKey>,
    created_at: String,
    volumes: &[SnapshotInput<'_>],
) -> CoreResult<(ContentId, u64)> {
    let mut volume_snapshots = Vec::with_capacity(volumes.len());
    let mut total_bytes = 0u64;

    for volume in volumes {
        let snapshot = ingest_volume_backup(store, key, volume.pvc_name, volume.data).await?;
        total_bytes += snapshot.bytes;
        volume_snapshots.push(snapshot);
    }

    let manifest = SnapshotManifest {
        version: MANIFEST_VERSION,
        created_at,
        encrypted: key.is_some(),
        volumes: volume_snapshots,
        total_bytes,
    };

    let id = seal_snapshot(store, &manifest).await?;
    Ok((id, total_bytes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::LocalBackend;

    #[tokio::test]
    async fn encrypted_ingest_stores_ciphertext_not_plaintext() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = LocalBackend::open(dir.path().to_str().expect("utf8"))
            .await
            .expect("open");
        let key = EncryptionKey::generate();
        let plaintext = b"very secret pvc contents".repeat(10);

        let snapshot = ingest_volume_backup(&store, Some(&key), "data", &plaintext)
            .await
            .expect("ingest");

        assert_eq!(snapshot.bytes, plaintext.len() as u64);
        assert_eq!(snapshot.chunk_ids.len(), 1);

        let id = ContentId::from_hex(&snapshot.chunk_ids[0]).expect("hex");
        let stored = store.get(&id).await.expect("get");
        assert_ne!(stored.as_ref(), plaintext.as_slice());

        let ciphertext = crate::crypto::Ciphertext {
            blob: stored.to_vec(),
        };
        let decrypted = crate::crypto::decrypt(&key, &ciphertext).expect("decrypt");
        assert_eq!(decrypted, plaintext);
    }

    #[tokio::test]
    async fn unencrypted_ingest_stores_plaintext() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = LocalBackend::open(dir.path().to_str().expect("utf8"))
            .await
            .expect("open");
        let plaintext = b"not secret";

        let snapshot = ingest_volume_backup(&store, None, "data", plaintext)
            .await
            .expect("ingest");

        let id = ContentId::from_hex(&snapshot.chunk_ids[0]).expect("hex");
        let stored = store.get(&id).await.expect("get");
        assert_eq!(stored.as_ref(), plaintext);
    }

    #[tokio::test]
    async fn create_snapshot_seals_manifest_and_reports_total_bytes() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = LocalBackend::open(dir.path().to_str().expect("utf8"))
            .await
            .expect("open");
        let key = EncryptionKey::generate();

        let a = b"pvc-a-bytes".to_vec();
        let b = b"pvc-b-bytes-longer".to_vec();
        let inputs = vec![
            SnapshotInput {
                pvc_name: "pvc-a",
                data: &a,
            },
            SnapshotInput {
                pvc_name: "pvc-b",
                data: &b,
            },
        ];

        let (id, total_bytes) = create_snapshot(
            &store,
            Some(&key),
            "2026-07-24T00:00:00Z".to_string(),
            &inputs,
        )
        .await
        .expect("create snapshot");

        assert_eq!(total_bytes, (a.len() + b.len()) as u64);

        let manifest_bytes = store.get(&id).await.expect("manifest stored");
        let manifest: SnapshotManifest =
            serde_json::from_slice(&manifest_bytes).expect("manifest json");
        assert!(manifest.encrypted);
        assert_eq!(manifest.volumes.len(), 2);
        assert_eq!(manifest.total_bytes, total_bytes);
    }
}

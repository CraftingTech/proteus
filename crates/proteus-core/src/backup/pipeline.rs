use bytes::Bytes;
use tokio::io::{AsyncRead, AsyncReadExt};

use super::snapshot::{SnapshotManifest, VolumeSnapshot, MANIFEST_VERSION};
use crate::chunking::{Chunker, DEFAULT_CHUNK_SIZE};
use crate::crypto::{decrypt, encrypt, Ciphertext, EncryptionKey};
use crate::error::{CoreError, CoreResult};
use crate::hash::{hash_bytes, ContentId};
use crate::storage::{ObjectStore, PutOptions};

/// Chunk puts use create-if-absent so identical content-addressed blobs dedup under concurrency.
const CHUNK_PUT: PutOptions = PutOptions {
    skip_if_exists: true,
};

/// One volume's raw plaintext bytes, ready to chunk and store.
pub struct SnapshotInput<'a> {
    pub pvc_name: &'a str,
    pub data: &'a [u8],
}

/// Chunk `data`, hash each chunk over plaintext, optionally encrypt, then `put` into `store`.
///
/// The chunk id is always the plaintext hash (dedup key + restore lookup); when `key` is set the
/// stored blob is the AES-GCM ciphertext under that same id, so blobs at rest are never plaintext.
///
/// `on_bytes`, when set, is called after each chunk with `(bytes_done, bytes_total)`.
pub async fn ingest_volume_backup(
    store: &dyn ObjectStore,
    key: Option<&EncryptionKey>,
    pvc_name: &str,
    data: &[u8],
) -> CoreResult<VolumeSnapshot> {
    ingest_volume_backup_with_progress(store, key, pvc_name, data, None).await
}

pub async fn ingest_volume_backup_with_progress(
    store: &dyn ObjectStore,
    key: Option<&EncryptionKey>,
    pvc_name: &str,
    data: &[u8],
    mut on_bytes: Option<&mut (dyn FnMut(u64, u64) + Send)>,
) -> CoreResult<VolumeSnapshot> {
    let chunker = Chunker::default();
    let chunks = chunker.chunk(data);
    let bytes_total = data.len() as u64;

    let mut chunk_ids = Vec::with_capacity(chunks.len());
    let mut bytes = 0u64;

    for chunk in chunks {
        bytes += chunk.data.len() as u64;
        let payload = match key {
            Some(key) => Bytes::from(encrypt(key, &chunk.data)?.blob),
            None => chunk.data,
        };
        store.put(&chunk.id, payload, CHUNK_PUT).await?;
        chunk_ids.push(chunk.id.to_hex());
        if let Some(cb) = on_bytes.as_mut() {
            cb(bytes, bytes_total);
        }
    }

    Ok(VolumeSnapshot {
        pvc_name: pvc_name.to_string(),
        bytes,
        chunk_ids,
    })
}

/// Stream `reader` into fixed-size CAS chunks without buffering the whole payload.
///
/// `on_bytes`, when set, is called after each chunk with bytes ingested so far (no total).
pub async fn ingest_volume_stream<R>(
    store: &dyn ObjectStore,
    key: Option<&EncryptionKey>,
    pvc_name: &str,
    mut reader: R,
    mut on_bytes: Option<&mut (dyn FnMut(u64) + Send)>,
) -> CoreResult<VolumeSnapshot>
where
    R: AsyncRead + Unpin + Send,
{
    let mut chunk_ids = Vec::new();
    let mut bytes = 0u64;
    let mut buf = vec![0u8; DEFAULT_CHUNK_SIZE];

    loop {
        let mut filled = 0usize;
        while filled < buf.len() {
            let n = reader.read(&mut buf[filled..]).await.map_err(|source| {
                CoreError::InvalidArgument(format!("volume stream read failed: {source}"))
            })?;
            if n == 0 {
                break;
            }
            filled += n;
        }
        if filled == 0 {
            break;
        }

        let plain = &buf[..filled];
        let id = hash_bytes(plain);
        let payload = match key {
            Some(key) => Bytes::from(encrypt(key, plain)?.blob),
            None => Bytes::copy_from_slice(plain),
        };
        store.put(&id, payload, CHUNK_PUT).await?;
        chunk_ids.push(id.to_hex());
        bytes += filled as u64;
        if let Some(cb) = on_bytes.as_mut() {
            cb(bytes);
        }
        if filled < buf.len() {
            break;
        }
    }

    Ok(VolumeSnapshot {
        pvc_name: pvc_name.to_string(),
        bytes,
        chunk_ids,
    })
}

/// Ingest one or more volume streams, seal the manifest, return id + total bytes.
pub async fn create_snapshot_from_streams<R>(
    store: &dyn ObjectStore,
    key: Option<&EncryptionKey>,
    created_at: String,
    volumes: &mut [(String, R)],
    mut on_bytes: Option<&mut (dyn FnMut(u64) + Send)>,
) -> CoreResult<(ContentId, u64)>
where
    R: AsyncRead + Unpin + Send,
{
    let mut volume_snapshots = Vec::with_capacity(volumes.len());
    let mut total_bytes = 0u64;
    let mut grand_done = 0u64;

    for (pvc_name, reader) in volumes.iter_mut() {
        let mut bridge = |done: u64| {
            grand_done = total_bytes + done;
            if let Some(cb) = on_bytes.as_mut() {
                cb(grand_done);
            }
        };
        let snap = ingest_volume_stream(
            store,
            key,
            pvc_name,
            reader,
            Some(&mut bridge as &mut (dyn FnMut(u64) + Send)),
        )
        .await?;
        total_bytes += snap.bytes;
        volume_snapshots.push(snap);
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
    create_snapshot_with_progress(store, key, created_at, volumes, None).await
}

/// Like [`create_snapshot`], with `on_bytes(done, total)` across all volume payloads.
pub async fn create_snapshot_with_progress(
    store: &dyn ObjectStore,
    key: Option<&EncryptionKey>,
    created_at: String,
    volumes: &[SnapshotInput<'_>],
    mut on_bytes: Option<&mut (dyn FnMut(u64, u64) + Send)>,
) -> CoreResult<(ContentId, u64)> {
    let mut volume_snapshots = Vec::with_capacity(volumes.len());
    let mut total_bytes = 0u64;
    let grand_total: u64 = volumes
        .iter()
        .map(|v| v.data.len() as u64)
        .sum::<u64>()
        .max(1);
    let mut grand_done = 0u64;
    let chunker = Chunker::default();

    for volume in volumes {
        let chunks = chunker.chunk(volume.data);
        let mut chunk_ids = Vec::with_capacity(chunks.len());
        let mut volume_bytes = 0u64;

        for chunk in chunks {
            let chunk_len = chunk.data.len() as u64;
            volume_bytes += chunk_len;
            let payload = match key {
                Some(key) => Bytes::from(encrypt(key, &chunk.data)?.blob),
                None => chunk.data,
            };
            store.put(&chunk.id, payload, CHUNK_PUT).await?;
            chunk_ids.push(chunk.id.to_hex());
            grand_done += chunk_len;
            if let Some(cb) = on_bytes.as_mut() {
                cb(grand_done.min(grand_total), grand_total);
            }
        }

        total_bytes += volume_bytes;
        volume_snapshots.push(VolumeSnapshot {
            pvc_name: volume.pvc_name.to_string(),
            bytes: volume_bytes,
            chunk_ids,
        });
    }

    let manifest = SnapshotManifest {
        version: MANIFEST_VERSION,
        created_at,
        encrypted: key.is_some(),
        volumes: volume_snapshots,
        total_bytes,
    };

    let id = seal_snapshot(store, &manifest).await?;
    if let Some(cb) = on_bytes.as_mut() {
        cb(grand_total, grand_total);
    }
    Ok((id, total_bytes))
}

/// Delete a sealed snapshot: every chunk referenced by the manifest, then the manifest itself.
///
/// Chunks shared with another snapshot (dedup) are still deleted — MVP has no refcounting.
/// Missing objects are ignored so delete stays idempotent.
pub async fn delete_snapshot(store: &dyn ObjectStore, id_hex: &str) -> CoreResult<()> {
    let manifest = match load_snapshot(store, id_hex).await {
        Ok(m) => m,
        Err(CoreError::NotFound(_)) => return Ok(()),
        Err(err) => return Err(err),
    };

    for volume in &manifest.volumes {
        for chunk_id_hex in &volume.chunk_ids {
            let id = ContentId::from_hex(chunk_id_hex)?;
            match store.delete(&id).await {
                Ok(()) | Err(CoreError::NotFound(_)) => {}
                Err(err) => return Err(err),
            }
        }
    }

    let manifest_id = ContentId::from_hex(id_hex)?;
    match store.delete(&manifest_id).await {
        Ok(()) | Err(CoreError::NotFound(_)) => Ok(()),
        Err(err) => Err(err),
    }
}

/// Delete every object in `store` that is not referenced by any of `keep_snapshot_ids`.
///
/// Used when removing a backup CR: keep snapshots belonging to *other* backups, drop the
/// deleted one plus any orphan blobs left by older deletes / interrupted runs.
/// Returns the number of objects removed.
pub async fn gc_unreferenced(
    store: &dyn ObjectStore,
    keep_snapshot_ids: &[String],
) -> CoreResult<u64> {
    use std::collections::HashSet;

    let mut keep: HashSet<ContentId> = HashSet::new();
    for id_hex in keep_snapshot_ids {
        let id_hex = id_hex.trim();
        if id_hex.is_empty() {
            continue;
        }
        let manifest_id = ContentId::from_hex(id_hex)?;
        keep.insert(manifest_id);
        match load_snapshot(store, id_hex).await {
            Ok(manifest) => {
                for volume in &manifest.volumes {
                    for chunk_id_hex in &volume.chunk_ids {
                        keep.insert(ContentId::from_hex(chunk_id_hex)?);
                    }
                }
            }
            Err(CoreError::NotFound(_)) => {}
            Err(err) => return Err(err),
        }
    }

    let listed = store.list_ids().await?;
    let mut deleted = 0u64;
    for id in listed {
        if keep.contains(&id) {
            continue;
        }
        store.delete(&id).await?;
        deleted += 1;
    }
    Ok(deleted)
}

/// `get` a sealed manifest by its content id (hex) and parse it back into a [`SnapshotManifest`].
pub async fn load_snapshot(store: &dyn ObjectStore, id_hex: &str) -> CoreResult<SnapshotManifest> {
    let id = ContentId::from_hex(id_hex)?;
    let bytes = store.get(&id).await?;
    let manifest: SnapshotManifest = serde_json::from_slice(&bytes)
        .map_err(|err| CoreError::InvalidArgument(format!("manifest deserialize: {err}")))?;
    if manifest.version != MANIFEST_VERSION {
        return Err(CoreError::InvalidArgument(format!(
            "unsupported snapshot manifest version {} (expected {MANIFEST_VERSION})",
            manifest.version
        )));
    }
    Ok(manifest)
}

/// Fetch every chunk of `volume` in order, decrypt when `key` is set, and concatenate back into
/// the original plaintext. Callers must pass `key` iff the manifest this volume came from was
/// sealed with `encrypted: true` — mismatching that flag fails with a crypto or format error.
pub async fn materialize_volume(
    store: &dyn ObjectStore,
    key: Option<&EncryptionKey>,
    volume: &VolumeSnapshot,
) -> CoreResult<Vec<u8>> {
    let mut out = Vec::with_capacity(volume.bytes as usize);

    for chunk_id_hex in &volume.chunk_ids {
        let id = ContentId::from_hex(chunk_id_hex)?;
        let stored = store.get(&id).await?;
        let plaintext = match key {
            Some(key) => decrypt(
                key,
                &Ciphertext {
                    blob: stored.to_vec(),
                },
            )?,
            None => stored.to_vec(),
        };
        out.extend_from_slice(&plaintext);
    }

    if out.len() as u64 != volume.bytes {
        return Err(CoreError::InvalidArgument(format!(
            "materialized {} bytes for pvc '{}', manifest recorded {}",
            out.len(),
            volume.pvc_name,
            volume.bytes
        )));
    }

    Ok(out)
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
    async fn second_ingest_identical_chunks_does_not_grow_store() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = LocalBackend::open(dir.path().to_str().expect("utf8"))
            .await
            .expect("open");
        let plaintext = b"dedup-me-across-backups";

        ingest_volume_backup(&store, None, "data", plaintext)
            .await
            .expect("first ingest");
        let after_first = store.list_ids().await.expect("list");
        assert_eq!(after_first.len(), 1);

        ingest_volume_backup(&store, None, "data", plaintext)
            .await
            .expect("second ingest");
        let after_second = store.list_ids().await.expect("list");
        assert_eq!(after_second.len(), after_first.len());

        let restored = materialize_volume(
            &store,
            None,
            &VolumeSnapshot {
                pvc_name: "data".into(),
                bytes: plaintext.len() as u64,
                chunk_ids: vec![after_first[0].to_hex()],
            },
        )
        .await
        .expect("materialize");
        assert_eq!(restored, plaintext);
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

    #[tokio::test]
    async fn load_snapshot_rejects_unknown_id() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = LocalBackend::open(dir.path().to_str().expect("utf8"))
            .await
            .expect("open");

        let err = load_snapshot(&store, &"ab".repeat(32))
            .await
            .expect_err("unknown id");
        assert!(matches!(err, CoreError::NotFound(_)));
    }

    #[tokio::test]
    async fn load_snapshot_rejects_bad_json() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = LocalBackend::open(dir.path().to_str().expect("utf8"))
            .await
            .expect("open");

        let garbage = Bytes::from_static(b"not a manifest");
        let id = hash_bytes(&garbage);
        store
            .put(&id, garbage, PutOptions::default())
            .await
            .expect("put garbage");

        let err = load_snapshot(&store, &id.to_hex())
            .await
            .expect_err("bad json");
        assert!(matches!(err, CoreError::InvalidArgument(_)));
    }

    #[tokio::test]
    async fn round_trip_create_load_materialize_plaintext() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = LocalBackend::open(dir.path().to_str().expect("utf8"))
            .await
            .expect("open");

        let data = b"plaintext pvc contents, chunked and reassembled".to_vec();
        let inputs = vec![SnapshotInput {
            pvc_name: "data",
            data: &data,
        }];

        let (id, _bytes) =
            create_snapshot(&store, None, "2026-07-24T00:00:00Z".to_string(), &inputs)
                .await
                .expect("create snapshot");

        let manifest = load_snapshot(&store, &id.to_hex())
            .await
            .expect("load snapshot");
        assert!(!manifest.encrypted);
        assert_eq!(manifest.volumes.len(), 1);

        let restored = materialize_volume(&store, None, &manifest.volumes[0])
            .await
            .expect("materialize");
        assert_eq!(restored, data);
    }

    #[tokio::test]
    async fn delete_snapshot_removes_manifest_and_chunks() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = LocalBackend::open(dir.path().to_str().expect("utf8"))
            .await
            .expect("open");

        let data = b"delete-me-contents".to_vec();
        let inputs = vec![SnapshotInput {
            pvc_name: "data",
            data: &data,
        }];
        let (id, _bytes) =
            create_snapshot(&store, None, "2026-07-24T00:00:00Z".to_string(), &inputs)
                .await
                .expect("create");
        let hex = id.to_hex();
        let manifest = load_snapshot(&store, &hex).await.expect("load");
        let chunk_hex = manifest.volumes[0].chunk_ids[0].clone();

        delete_snapshot(&store, &hex).await.expect("delete");
        assert!(load_snapshot(&store, &hex).await.is_err());
        let chunk_id = ContentId::from_hex(&chunk_hex).expect("hex");
        assert!(store.get(&chunk_id).await.is_err());
        // Idempotent.
        delete_snapshot(&store, &hex).await.expect("delete again");
    }

    #[tokio::test]
    async fn stream_ingest_matches_buffered_round_trip() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = LocalBackend::open(dir.path().to_str().expect("utf8"))
            .await
            .expect("open");
        let data = b"streamed-pvc-bytes-for-parity-check-0123456789".repeat(40);

        let (id_buf, bytes_buf) = create_snapshot(
            &store,
            None,
            "2026-07-24T00:00:00Z".to_string(),
            &[SnapshotInput {
                pvc_name: "data",
                data: &data,
            }],
        )
        .await
        .expect("buffered");

        let dir2 = tempfile::tempdir().expect("tempdir2");
        let store2 = LocalBackend::open(dir2.path().to_str().expect("utf8"))
            .await
            .expect("open2");
        let mut volumes = vec![("data".to_string(), data.as_slice())];
        let (id_stream, bytes_stream) = create_snapshot_from_streams(
            &store2,
            None,
            "2026-07-24T00:00:00Z".to_string(),
            &mut volumes,
            None,
        )
        .await
        .expect("stream");

        assert_eq!(bytes_buf, bytes_stream);
        assert_eq!(id_buf, id_stream);
        let restored = materialize_volume(
            &store2,
            None,
            &load_snapshot(&store2, &id_stream.to_hex())
                .await
                .expect("load")
                .volumes[0],
        )
        .await
        .expect("materialize");
        assert_eq!(restored, data);
    }

    #[tokio::test]
    async fn gc_unreferenced_keeps_other_snapshots_drops_orphans() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = LocalBackend::open(dir.path().to_str().expect("utf8"))
            .await
            .expect("open");

        let a = b"snapshot-a-payload".to_vec();
        let b = b"snapshot-b-payload-different".to_vec();
        let (id_a, _) = create_snapshot(
            &store,
            None,
            "2026-07-24T00:00:00Z".to_string(),
            &[SnapshotInput {
                pvc_name: "a",
                data: &a,
            }],
        )
        .await
        .expect("a");
        let (id_b, _) = create_snapshot(
            &store,
            None,
            "2026-07-24T01:00:00Z".to_string(),
            &[SnapshotInput {
                pvc_name: "b",
                data: &b,
            }],
        )
        .await
        .expect("b");

        // Orphan blob not referenced by either snapshot.
        let orphan = hash_bytes(b"orphan-blob");
        store
            .put(
                &orphan,
                bytes::Bytes::from_static(b"orphan-blob"),
                crate::storage::PutOptions::default(),
            )
            .await
            .expect("orphan");

        let removed = gc_unreferenced(&store, &[id_a.to_hex()]).await.expect("gc");
        assert!(removed >= 2); // at least snapshot B + orphan
        assert!(load_snapshot(&store, &id_a.to_hex()).await.is_ok());
        assert!(load_snapshot(&store, &id_b.to_hex()).await.is_err());
        assert!(store.get(&orphan).await.is_err());
    }

    #[tokio::test]
    async fn round_trip_create_load_materialize_encrypted() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = LocalBackend::open(dir.path().to_str().expect("utf8"))
            .await
            .expect("open");
        let key = EncryptionKey::generate();

        let data = b"encrypted pvc contents, chunked and reassembled".repeat(4);
        let inputs = vec![SnapshotInput {
            pvc_name: "data",
            data: &data,
        }];

        let (id, _bytes) = create_snapshot(
            &store,
            Some(&key),
            "2026-07-24T00:00:00Z".to_string(),
            &inputs,
        )
        .await
        .expect("create snapshot");

        let manifest = load_snapshot(&store, &id.to_hex())
            .await
            .expect("load snapshot");
        assert!(manifest.encrypted);

        let restored = materialize_volume(&store, Some(&key), &manifest.volumes[0])
            .await
            .expect("materialize");
        assert_eq!(restored, data);
    }

    #[tokio::test]
    async fn round_trip_multi_volume_snapshot() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = LocalBackend::open(dir.path().to_str().expect("utf8"))
            .await
            .expect("open");
        let key = EncryptionKey::generate();

        let a = b"volume-a-bytes".to_vec();
        let b = b"volume-b-bytes-longer-than-a".to_vec();
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

        let (id, _bytes) = create_snapshot(
            &store,
            Some(&key),
            "2026-07-24T00:00:00Z".to_string(),
            &inputs,
        )
        .await
        .expect("create snapshot");

        let manifest = load_snapshot(&store, &id.to_hex())
            .await
            .expect("load snapshot");

        let restored_a = materialize_volume(&store, Some(&key), &manifest.volumes[0])
            .await
            .expect("materialize a");
        let restored_b = materialize_volume(&store, Some(&key), &manifest.volumes[1])
            .await
            .expect("materialize b");
        assert_eq!(restored_a, a);
        assert_eq!(restored_b, b);
    }

    #[tokio::test]
    async fn materialize_volume_rejects_size_mismatch() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = LocalBackend::open(dir.path().to_str().expect("utf8"))
            .await
            .expect("open");

        let snapshot = ingest_volume_backup(&store, None, "data", b"short")
            .await
            .expect("ingest");
        let mut tampered = snapshot;
        tampered.bytes += 1; // manifest now disagrees with the stored chunk length

        let err = materialize_volume(&store, None, &tampered)
            .await
            .expect_err("size mismatch");
        assert!(matches!(err, CoreError::InvalidArgument(_)));
    }
}

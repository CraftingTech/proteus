//! Controller-side helpers for running a `ProteusBackup`: mount+exec PVC reads and repository
//! access. The actual chunk/encrypt/store pipeline lives in `proteus_core::backup`.

pub mod progress;
pub mod pvc_reader;
pub mod repo;

use std::sync::Arc;

use kube::{Resource, ResourceExt};
use proteus_core::backup::{seal_snapshot, SnapshotManifest, MANIFEST_VERSION};
use proteus_crd::ProteusBackup;

use self::progress::{format_bytes, map_range, BackupProgressSink};
use self::pvc_reader::{BackupMountOwner, MountStage};
use self::repo::open_repository;
use crate::controllers::ReconcileCtx;

/// Stream every PVC named in `backup.spec.pvc_names` into the repository (no full-archive buffer).
///
/// Progress bands (single PVC):
/// - 0–5% open repository
/// - 6–9% create/wait mount Pod + `du`
/// - 10–98% stream tar → chunk → put
/// - 100% sealed (set by controller on Succeeded)
pub async fn run_backup(
    backup: &ProteusBackup,
    ctx: &ReconcileCtx,
    backup_namespace: &str,
    progress: Arc<BackupProgressSink>,
) -> Result<(String, u64), String> {
    progress.report(1, "opening repository".to_string()).await?;

    let opened = open_repository(
        &ctx.client,
        backup_namespace,
        &backup.spec.repository_ref,
        backup.spec.repository_namespace.as_deref(),
    )
    .await?;

    progress.report(5, "repository ready".to_string()).await?;

    let owner = BackupMountOwner {
        backup_name: backup.name_any(),
        backup_namespace: backup_namespace.to_string(),
        backup_uid: backup.meta().uid.clone().unwrap_or_default(),
    };

    // Drop pods left by a previous interrupted run / controller restart.
    if let Err(err) = pvc_reader::cleanup_backup_mount_pods(
        &ctx.client,
        &backup.spec.target_namespace,
        &owner.backup_name,
        &owner.backup_namespace,
    )
    .await
    {
        tracing::warn!(error = %err, "failed to cleanup leftover backup mount pods");
    }

    let pvc_names = &backup.spec.pvc_names;
    let pvc_count = pvc_names.len().max(1);
    let mut volume_snapshots = Vec::with_capacity(pvc_names.len());
    let mut total_bytes = 0u64;

    for (index, pvc_name) in pvc_names.iter().enumerate() {
        let range_start = 10 + ((index * 88) / pvc_count) as u8;
        let range_end = 10 + (((index + 1) * 88) / pvc_count) as u8;
        let progress = Arc::clone(&progress);
        let pvc_label = pvc_name.clone();

        let snap = {
            let progress_stage = Arc::clone(&progress);
            let progress_bytes = Arc::clone(&progress);
            let pvc_for_stage = pvc_label.clone();
            let pvc_for_cb = pvc_label.clone();
            pvc_reader::stream_pvc_into_store(
                &ctx.client,
                &backup.spec.target_namespace,
                pvc_name,
                &owner,
                opened.store.as_ref(),
                opened.encryption_key.as_ref(),
                move |stage| {
                    let progress = Arc::clone(&progress_stage);
                    let pvc = pvc_for_stage.clone();
                    async move {
                        let (pct, msg) = match stage {
                            MountStage::CreatingPod => {
                                (6u8, format!("PVC '{pvc}': creating mount pod"))
                            }
                            MountStage::WaitingPodRunning => (
                                7,
                                format!(
                                    "PVC '{pvc}': waiting for mount pod (pull image / attach volume)"
                                ),
                            ),
                            MountStage::MeasuringSize => {
                                (8, format!("PVC '{pvc}': measuring volume size"))
                            }
                            MountStage::StreamingTar => {
                                (10, format!("PVC '{pvc}': streaming to repository"))
                            }
                        };
                        progress.report(pct, msg).await
                    }
                },
                move |read, total_hint| {
                    let progress = Arc::clone(&progress_bytes);
                    let pvc_for_cb = pvc_for_cb.clone();
                    async move {
                        let pct = match total_hint.filter(|t| *t > 0) {
                            Some(total) => map_range(range_start, range_end, read, total),
                            None => {
                                let span = range_end.saturating_sub(range_start).max(1);
                                let creep = ((read / (4 * 1024 * 1024)) as u8)
                                    .min(span.saturating_sub(1));
                                range_start + creep
                            }
                        };
                        let msg = match total_hint.filter(|t| *t > 0) {
                            Some(total) => format!(
                                "PVC '{pvc_for_cb}': {} / {} → repository",
                                format_bytes(read),
                                format_bytes(total)
                            ),
                            None => format!(
                                "PVC '{pvc_for_cb}': {} → repository",
                                format_bytes(read)
                            ),
                        };
                        progress.report(pct, msg).await
                    }
                },
            )
            .await
            .map_err(|err| format!("PVC '{pvc_label}': {err}"))?
        };

        total_bytes += snap.bytes;
        progress
            .report(
                range_end,
                format!("PVC '{pvc_label}': stored ({})", format_bytes(snap.bytes)),
            )
            .await?;
        volume_snapshots.push(snap);

        if progress.is_cancelled() {
            return Err("backup deleted; cancelling".into());
        }
    }

    progress.report(99, "sealing snapshot".to_string()).await?;

    let manifest = SnapshotManifest {
        version: MANIFEST_VERSION,
        created_at: chrono::Utc::now().to_rfc3339(),
        encrypted: opened.encryption_key.is_some(),
        volumes: volume_snapshots,
        total_bytes,
    };
    let id = seal_snapshot(opened.store.as_ref(), &manifest)
        .await
        .map_err(|err| format!("seal snapshot failed: {err}"))?;

    Ok((id.to_hex(), total_bytes))
}

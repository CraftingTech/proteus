//! Controller-side helpers for running a `ProteusRestore`: resolve the source backup + snapshot,
//! open its repository, decrypt via the repository key, and write each volume's tar bytes into
//! the matching PVC in the target namespace. Mirrors `backup::run_backup`.

pub mod pvc_writer;

use kube::{Api, ResourceExt};
use proteus_crd::{BackupPhase, ProteusBackup, ProteusRestore};

use crate::backup::repo::open_repository;
use crate::controllers::ReconcileCtx;

/// Run one restore end to end. Returns the snapshot id that was restored.
pub async fn run_restore(
    restore: &ProteusRestore,
    ctx: &ReconcileCtx,
    restore_namespace: &str,
) -> Result<String, String> {
    let backup = resolve_backup(ctx, restore, restore_namespace).await?;
    let snapshot_id = resolve_snapshot_id(restore, &backup)?;
    let backup_namespace = backup
        .namespace()
        .unwrap_or_else(|| restore_namespace.to_string());

    let opened = open_repository(
        &ctx.client,
        &backup_namespace,
        &backup.spec.repository_ref,
        backup.spec.repository_namespace.as_deref(),
    )
    .await?;

    let manifest = proteus_core::backup::load_snapshot(opened.store.as_ref(), &snapshot_id)
        .await
        .map_err(|err| format!("failed to load snapshot '{snapshot_id}': {err}"))?;

    if manifest.encrypted && opened.encryption_key.is_none() {
        return Err(format!(
            "snapshot '{snapshot_id}' is encrypted but repository '{}' has no key configured",
            backup.spec.repository_ref
        ));
    }
    if manifest.volumes.is_empty() {
        return Err(format!(
            "snapshot '{snapshot_id}' contains no volumes to restore"
        ));
    }

    // Only decrypt when the manifest says the chunks are ciphertext — a key present on a
    // repository that later enabled encryption must not be applied to older plaintext chunks.
    let decrypt_key = if manifest.encrypted {
        opened.encryption_key.as_ref()
    } else {
        None
    };

    for volume in &manifest.volumes {
        let data =
            proteus_core::backup::materialize_volume(opened.store.as_ref(), decrypt_key, volume)
                .await
                .map_err(|err| {
                    format!(
                        "PVC '{}': failed to materialize snapshot data: {err}",
                        volume.pvc_name
                    )
                })?;

        pvc_writer::write_pvc_tar(
            &ctx.client,
            &restore.spec.target_namespace,
            &volume.pvc_name,
            restore.spec.overwrite,
            data,
        )
        .await
        .map_err(|err| format!("PVC '{}': {err}", volume.pvc_name))?;
    }

    Ok(snapshot_id)
}

/// Load the source `ProteusBackup` (defaulting to the restore's own namespace) and require it be
/// `Succeeded` — restoring from a backup that never finished would restore garbage or nothing.
async fn resolve_backup(
    ctx: &ReconcileCtx,
    restore: &ProteusRestore,
    restore_namespace: &str,
) -> Result<ProteusBackup, String> {
    let backup_ns = restore
        .spec
        .backup_namespace
        .as_deref()
        .unwrap_or(restore_namespace);
    let api: Api<ProteusBackup> = Api::namespaced(ctx.client.clone(), backup_ns);
    let backup = api.get(&restore.spec.backup_ref).await.map_err(|err| {
        format!(
            "backup '{}' not found in namespace '{backup_ns}': {err}",
            restore.spec.backup_ref
        )
    })?;

    let phase = backup.status.as_ref().and_then(|s| s.phase.clone());
    if !matches!(phase, Some(BackupPhase::Succeeded)) {
        return Err(format!(
            "backup '{}' in namespace '{backup_ns}' is not Succeeded (phase={phase:?})",
            restore.spec.backup_ref
        ));
    }
    Ok(backup)
}

/// `spec.snapshotId` wins when set; otherwise fall back to the backup's latest snapshot.
fn resolve_snapshot_id(restore: &ProteusRestore, backup: &ProteusBackup) -> Result<String, String> {
    restore
        .spec
        .snapshot_id
        .clone()
        .filter(|id| !id.trim().is_empty())
        .or_else(|| {
            backup
                .status
                .as_ref()
                .and_then(|s| s.last_snapshot_id.clone())
        })
        .ok_or_else(|| {
            format!(
                "backup '{}' has no snapshot id (set spec.snapshotId, or wait for a successful backup run)",
                restore.spec.backup_ref
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use proteus_crd::{ProteusBackupStatus, ProteusRestoreSpec};

    fn restore_with(snapshot_id: Option<&str>) -> ProteusRestore {
        ProteusRestore::new(
            "test-restore",
            ProteusRestoreSpec {
                backup_ref: "backup-1".to_string(),
                backup_namespace: None,
                snapshot_id: snapshot_id.map(str::to_string),
                target_namespace: "workloads".to_string(),
                overwrite: false,
                include_resources: None,
            },
        )
    }

    fn backup_with_snapshot(snapshot_id: Option<&str>) -> ProteusBackup {
        let mut backup = ProteusBackup::new(
            "backup-1",
            proteus_crd::ProteusBackupSpec {
                policy_ref: None,
                policy_namespace: None,
                repository_ref: "repo".to_string(),
                repository_namespace: None,
                target_namespace: "workloads".to_string(),
                pvc_names: vec!["data".to_string()],
                label_selector: None,
                schedule: None,
                retention: proteus_crd::RetentionPolicy::default(),
                include_volumes: true,
                include_cluster_resources: false,
            },
        );
        backup.status = Some(ProteusBackupStatus {
            last_snapshot_id: snapshot_id.map(str::to_string),
            ..Default::default()
        });
        backup
    }

    #[test]
    fn resolve_snapshot_id_prefers_spec_over_backup_status() {
        let restore = restore_with(Some("from-spec"));
        let backup = backup_with_snapshot(Some("from-backup"));
        assert_eq!(
            resolve_snapshot_id(&restore, &backup).expect("resolved"),
            "from-spec"
        );
    }

    #[test]
    fn resolve_snapshot_id_falls_back_to_backup_last_snapshot() {
        let restore = restore_with(None);
        let backup = backup_with_snapshot(Some("from-backup"));
        assert_eq!(
            resolve_snapshot_id(&restore, &backup).expect("resolved"),
            "from-backup"
        );
    }

    #[test]
    fn resolve_snapshot_id_rejects_missing_snapshot() {
        let restore = restore_with(None);
        let backup = backup_with_snapshot(None);
        let err = resolve_snapshot_id(&restore, &backup).expect_err("no snapshot id anywhere");
        assert!(err.contains("snapshot id"));
    }

    #[test]
    fn resolve_snapshot_id_ignores_blank_spec_value() {
        let restore = restore_with(Some("   "));
        let backup = backup_with_snapshot(Some("from-backup"));
        assert_eq!(
            resolve_snapshot_id(&restore, &backup).expect("resolved"),
            "from-backup"
        );
    }
}

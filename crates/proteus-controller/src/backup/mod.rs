//! Controller-side helpers for running a `ProteusBackup`: mount+exec PVC reads and repository
//! access. The actual chunk/encrypt/store pipeline lives in `proteus_core::backup`.

pub mod pvc_reader;
pub mod repo;

use proteus_core::backup::SnapshotInput;
use proteus_crd::ProteusBackup;

use self::repo::open_repository;
use crate::controllers::ReconcileCtx;

/// Read every PVC named in `backup.spec.pvc_names`, open the target repository, and run the
/// snapshot pipeline. Returns the sealed snapshot id (hex) and total plaintext bytes.
pub async fn run_backup(
    backup: &ProteusBackup,
    ctx: &ReconcileCtx,
    backup_namespace: &str,
) -> Result<(String, u64), String> {
    let opened = open_repository(
        ctx,
        backup_namespace,
        &backup.spec.repository_ref,
        backup.spec.repository_namespace.as_deref(),
    )
    .await?;

    let mut volumes = Vec::with_capacity(backup.spec.pvc_names.len());
    for pvc_name in &backup.spec.pvc_names {
        let data = pvc_reader::read_pvc_tar(&ctx.client, &backup.spec.target_namespace, pvc_name)
            .await
            .map_err(|err| format!("PVC '{pvc_name}': {err}"))?;
        volumes.push((pvc_name.clone(), data));
    }

    let inputs: Vec<SnapshotInput<'_>> = volumes
        .iter()
        .map(|(name, data)| SnapshotInput {
            pvc_name: name,
            data,
        })
        .collect();

    let (id, bytes) = proteus_core::backup::create_snapshot(
        opened.store.as_ref(),
        opened.encryption_key.as_ref(),
        chrono::Utc::now().to_rfc3339(),
        &inputs,
    )
    .await
    .map_err(|err| format!("snapshot pipeline failed: {err}"))?;

    Ok((id.to_hex(), bytes))
}

//! `proteus-controller mover` — runs inside a short-lived Pod with PVCs mounted
//! under `/volumes/<pvcName>` and performs CAS ingest or extract without crossing
//! the apiserver data plane.

use std::path::PathBuf;
use std::process::Stdio;

use anyhow::{bail, Context, Result};
use chrono::Utc;
use kube::api::{Patch, PatchParams};
use kube::{Api, Client};
use proteus_core::backup::{
    ingest_volume_stream, load_snapshot, materialize_volume_to_writer, seal_snapshot,
    SnapshotManifest, MANIFEST_VERSION,
};
use proteus_crd::{
    BackupPhase, DataPlane, ProteusBackup, ProteusBackupStatus, ProteusRestore,
    ProteusRestoreStatus, RestorePhase,
};
use tokio::process::Command;
use tracing::info;

use crate::backup::recipe::load_recipe;
use crate::backup::repo::open_repository;

const VOLUME_ROOT: &str = "/volumes";

pub async fn run(args: &[String]) -> Result<()> {
    let mut rest = args.iter().map(String::as_str);
    let Some(kind) = rest.next() else {
        bail!("usage: proteus-controller mover <backup|restore> --namespace <ns> --name <name>");
    };
    let mut namespace = None;
    let mut name = None;
    while let Some(flag) = rest.next() {
        match flag {
            "--namespace" | "-n" => {
                namespace = rest.next().map(str::to_string);
            }
            "--name" => {
                name = rest.next().map(str::to_string);
            }
            other => bail!("unknown mover flag {other}"),
        }
    }
    let namespace = namespace.context("--namespace is required")?;
    let name = name.context("--name is required")?;

    match kind {
        "backup" => run_backup_mover(&namespace, &name).await,
        "restore" => run_restore_mover(&namespace, &name).await,
        other => bail!("unknown mover kind {other:?}; expected backup or restore"),
    }
}

async fn run_backup_mover(namespace: &str, name: &str) -> Result<()> {
    let client = Client::try_default().await.context("kube client")?;
    let api: Api<ProteusBackup> = Api::namespaced(client.clone(), namespace);
    let backup = api.get(name).await.context("get ProteusBackup")?;
    let recipe = load_recipe(&client, &backup, namespace)
        .await
        .map_err(anyhow::Error::msg)?;

    let started_at = backup
        .status
        .as_ref()
        .and_then(|s| s.started_at.clone())
        .unwrap_or_else(|| Utc::now().to_rfc3339());

    let opened = open_repository(
        &client,
        namespace,
        &recipe.repository_ref,
        recipe.repository_namespace.as_deref(),
    )
    .await
    .map_err(anyhow::Error::msg)?;

    let mut volume_snapshots = Vec::with_capacity(recipe.pvc_names.len());
    let mut total_bytes = 0u64;

    for pvc_name in &recipe.pvc_names {
        let mount = PathBuf::from(VOLUME_ROOT).join(pvc_name);
        if !mount.is_dir() {
            bail!("expected PVC mount directory {}", mount.display());
        }
        info!(%pvc_name, path = %mount.display(), "mover ingesting volume");
        patch_backup_message(
            &api,
            name,
            &backup,
            &format!("agent mover: streaming PVC '{pvc_name}'"),
            20,
        )
        .await?;

        let mut child = Command::new("tar")
            .args(["-cf", "-", "-C"])
            .arg(&mount)
            .arg(".")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .context("spawn tar")?;
        let stdout = child.stdout.take().context("tar stdout")?;
        let snap = ingest_volume_stream(
            opened.store.as_ref(),
            opened.encryption_key.as_ref(),
            pvc_name,
            stdout,
            None,
        )
        .await
        .map_err(|err| anyhow::anyhow!("ingest {pvc_name}: {err}"))?;

        let status = child.wait().await.context("wait tar")?;
        if !status.success() {
            let mut err = child.stderr.take();
            let mut buf = String::new();
            if let Some(mut stderr) = err.take() {
                let _ = tokio::io::AsyncReadExt::read_to_string(&mut stderr, &mut buf).await;
            }
            bail!("tar failed for PVC '{pvc_name}' (status={status}): {buf}");
        }

        total_bytes += snap.bytes;
        volume_snapshots.push(snap);
    }

    let manifest = SnapshotManifest {
        version: MANIFEST_VERSION,
        created_at: Utc::now().to_rfc3339(),
        encrypted: opened.encryption_key.is_some(),
        volumes: volume_snapshots,
        total_bytes,
    };
    let snapshot_id = seal_snapshot(opened.store.as_ref(), &manifest)
        .await
        .map_err(|err| anyhow::anyhow!("seal snapshot: {err}"))?;

    let finished = Utc::now();
    let duration_seconds = chrono::DateTime::parse_from_rfc3339(&started_at)
        .ok()
        .map(|s| {
            finished
                .signed_duration_since(s.with_timezone(&Utc))
                .num_seconds()
                .max(1) as u64
        });
    let throughput = duration_seconds.map(|d| total_bytes / d);

    let status = ProteusBackupStatus {
        phase: Some(BackupPhase::Succeeded),
        message: Some(format!(
            "backup succeeded via agent mover ({total_bytes} bytes)"
        )),
        last_snapshot_id: Some(snapshot_id.to_hex()),
        last_success_at: Some(finished.to_rfc3339()),
        last_bytes: Some(total_bytes),
        progress_percent: Some(100),
        started_at: Some(started_at),
        duration_seconds,
        throughput_bytes_per_sec: throughput,
        data_plane: Some(DataPlane::Agent),
        assigned_node: backup.status.as_ref().and_then(|s| s.assigned_node.clone()),
        ..Default::default()
    };
    patch_backup_status(&api, name, &status).await?;
    info!(%name, bytes = total_bytes, "agent backup mover finished");
    Ok(())
}

async fn run_restore_mover(namespace: &str, name: &str) -> Result<()> {
    let client = Client::try_default().await.context("kube client")?;
    let api: Api<ProteusRestore> = Api::namespaced(client.clone(), namespace);
    let restore = api.get(name).await.context("get ProteusRestore")?;

    let started_at = restore
        .status
        .as_ref()
        .and_then(|s| s.started_at.clone())
        .unwrap_or_else(|| Utc::now().to_rfc3339());

    // Reuse controller helpers via a minimal ReconcileCtx-free path.
    let backup_ns = restore
        .spec
        .backup_namespace
        .as_deref()
        .unwrap_or(namespace);
    let backups: Api<ProteusBackup> = Api::namespaced(client.clone(), backup_ns);
    let backup = backups
        .get(&restore.spec.backup_ref)
        .await
        .with_context(|| format!("get backup {}", restore.spec.backup_ref))?;
    let snapshot_id = restore
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
        .context("no snapshot id on restore or backup")?;

    let recipe = load_recipe(&client, &backup, backup_ns)
        .await
        .map_err(anyhow::Error::msg)?;
    let opened = open_repository(
        &client,
        backup_ns,
        &recipe.repository_ref,
        recipe.repository_namespace.as_deref(),
    )
    .await
    .map_err(anyhow::Error::msg)?;

    let manifest = load_snapshot(opened.store.as_ref(), &snapshot_id)
        .await
        .map_err(|err| anyhow::anyhow!("load snapshot: {err}"))?;
    let decrypt_key = if manifest.encrypted {
        opened.encryption_key.as_ref()
    } else {
        None
    };

    let mut total_bytes = 0u64;
    for volume in &manifest.volumes {
        let mount = PathBuf::from(VOLUME_ROOT).join(&volume.pvc_name);
        if !mount.is_dir() {
            bail!("expected PVC mount directory {}", mount.display());
        }
        info!(pvc = %volume.pvc_name, "mover restoring volume");
        if restore.spec.overwrite {
            clear_dir(&mount).await?;
        } else if !dir_is_empty(&mount).await? {
            bail!(
                "target PVC '{}' already has data (set overwrite: true)",
                volume.pvc_name
            );
        }

        let mut child = Command::new("tar")
            .args(["-xf", "-", "-C"])
            .arg(&mount)
            .stdin(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .context("spawn tar extract")?;
        let stdin = child.stdin.take().context("tar stdin")?;
        let written = materialize_volume_to_writer(opened.store.as_ref(), decrypt_key, volume, stdin)
            .await
            .map_err(|err| anyhow::anyhow!("materialize {}: {err}", volume.pvc_name))?;
        total_bytes += written;

        let status = child.wait().await.context("wait tar extract")?;
        if !status.success() {
            bail!("tar extract failed for PVC '{}'", volume.pvc_name);
        }
    }

    let finished = Utc::now();
    let duration_seconds = chrono::DateTime::parse_from_rfc3339(&started_at)
        .ok()
        .map(|s| {
            finished
                .signed_duration_since(s.with_timezone(&Utc))
                .num_seconds()
                .max(1) as u64
        });
    let throughput = duration_seconds.map(|d| total_bytes / d);

    let status = ProteusRestoreStatus {
        phase: Some(RestorePhase::Succeeded),
        message: Some(format!(
            "restore succeeded via agent mover from snapshot {snapshot_id}"
        )),
        restored_snapshot_id: Some(snapshot_id),
        progress_percent: Some(100),
        completed_at: Some(finished.to_rfc3339()),
        started_at: Some(started_at),
        last_bytes: Some(total_bytes),
        duration_seconds,
        throughput_bytes_per_sec: throughput,
        data_plane: Some(DataPlane::Agent),
        assigned_node: restore.status.as_ref().and_then(|s| s.assigned_node.clone()),
    };
    let patch = serde_json::json!({ "status": status });
    api.patch_status(name, &PatchParams::default(), &Patch::Merge(&patch))
        .await
        .context("patch restore status")?;
    info!(%name, bytes = total_bytes, "agent restore mover finished");
    Ok(())
}

async fn clear_dir(path: &PathBuf) -> Result<()> {
    let mut entries = tokio::fs::read_dir(path).await.context("read_dir")?;
    while let Some(entry) = entries.next_entry().await.context("next_entry")? {
        let p = entry.path();
        if entry.file_type().await?.is_dir() {
            tokio::fs::remove_dir_all(&p).await?;
        } else {
            tokio::fs::remove_file(&p).await?;
        }
    }
    Ok(())
}

async fn dir_is_empty(path: &PathBuf) -> Result<bool> {
    let mut entries = tokio::fs::read_dir(path).await.context("read_dir")?;
    Ok(entries.next_entry().await.context("next_entry")?.is_none())
}

async fn patch_backup_message(
    api: &Api<ProteusBackup>,
    name: &str,
    backup: &ProteusBackup,
    message: &str,
    progress: u8,
) -> Result<()> {
    let mut status = backup.status.clone().unwrap_or_default();
    status.phase = Some(BackupPhase::Running);
    status.message = Some(message.to_string());
    status.progress_percent = Some(progress);
    status.data_plane = Some(DataPlane::Agent);
    patch_backup_status(api, name, &status).await
}

async fn patch_backup_status(
    api: &Api<ProteusBackup>,
    name: &str,
    status: &ProteusBackupStatus,
) -> Result<()> {
    let patch = serde_json::json!({ "status": status });
    api.patch_status(name, &PatchParams::default(), &Patch::Merge(&patch))
        .await
        .with_context(|| format!("patch backup status {name}"))?;
    Ok(())
}

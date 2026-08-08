//! Poll for Backups/Restores assigned to this node and run mover Pods.

use std::collections::{BTreeMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use k8s_openapi::api::core::v1::{
    Container, EnvVar, PersistentVolumeClaimVolumeSource, Pod, PodSpec, Toleration, Volume,
    VolumeMount,
};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
use kube::api::{DeleteParams, ListParams, Patch, PatchParams, PostParams};
use kube::{Api, Client, ResourceExt};
use proteus_crd::{BackupPhase, DataPlane, ProteusBackup, ProteusRestore, RestorePhase};
use tokio::sync::Mutex;
use tracing::{info, warn};

use super::identity::MOVER_SA;

const LABEL_PURPOSE: &str = "proteus.io/purpose";
const PURPOSE_BACKUP_MOVER: &str = "backup-mover";
const PURPOSE_RESTORE_MOVER: &str = "restore-mover";
const LABEL_OWNER: &str = "proteus.io/owner";
const LABEL_OWNER_NS: &str = "proteus.io/owner-namespace";
const VOLUME_ROOT: &str = "/volumes";
const MOVER_POD_TIMEOUT: Duration = Duration::from_secs(6 * 60 * 60);
const POLL: Duration = Duration::from_secs(5);

/// Poll assigned CRs and spawn movers until the process exits.
pub async fn run_work_loop(client: Client, node_name: String, mover_image: String) -> Result<()> {
    let inflight: Arc<Mutex<HashSet<String>>> = Arc::new(Mutex::new(HashSet::new()));
    let image = Arc::new(mover_image);
    loop {
        if let Err(err) = poll_once(&client, &node_name, &inflight, &image).await {
            warn!(error = %err, "agent work poll failed");
        }
        tokio::time::sleep(POLL).await;
    }
}

async fn poll_once(
    client: &Client,
    node_name: &str,
    inflight: &Arc<Mutex<HashSet<String>>>,
    image: &Arc<String>,
) -> Result<()> {
    let backups: Api<ProteusBackup> = Api::all(client.clone());
    for backup in backups.list(&ListParams::default()).await?.items {
        if !is_assigned_backup(&backup, node_name) {
            continue;
        }
        let ns = backup.namespace().unwrap_or_else(|| "default".into());
        let name = backup.name_any();
        let key = format!("backup:{ns}/{name}");
        {
            let mut guard = inflight.lock().await;
            if !guard.insert(key.clone()) {
                continue;
            }
        }
        let client = client.clone();
        let inflight = Arc::clone(inflight);
        let image = Arc::clone(image);
        tokio::spawn(async move {
            if let Err(err) = run_backup_mover_pod(&client, &ns, &backup, &image).await {
                warn!(%ns, %name, error = %err, "backup mover failed");
                let _ = fail_backup(&client, &ns, &name, &err.to_string()).await;
            }
            inflight.lock().await.remove(&key);
        });
    }

    let restores: Api<ProteusRestore> = Api::all(client.clone());
    for restore in restores.list(&ListParams::default()).await?.items {
        if !is_assigned_restore(&restore, node_name) {
            continue;
        }
        let ns = restore.namespace().unwrap_or_else(|| "default".into());
        let name = restore.name_any();
        let key = format!("restore:{ns}/{name}");
        {
            let mut guard = inflight.lock().await;
            if !guard.insert(key.clone()) {
                continue;
            }
        }
        let client = client.clone();
        let inflight = Arc::clone(inflight);
        let image = Arc::clone(image);
        tokio::spawn(async move {
            if let Err(err) = run_restore_mover_pod(&client, &ns, &restore, &image).await {
                warn!(%ns, %name, error = %err, "restore mover failed");
                let _ = fail_restore(&client, &ns, &name, &err.to_string()).await;
            }
            inflight.lock().await.remove(&key);
        });
    }
    Ok(())
}

fn is_assigned_backup(backup: &ProteusBackup, node_name: &str) -> bool {
    let Some(status) = backup.status.as_ref() else {
        return false;
    };
    status.data_plane.as_ref() == Some(&DataPlane::Agent)
        && status.assigned_node.as_deref() == Some(node_name)
        && status.phase.as_ref() == Some(&BackupPhase::Running)
        && status.last_snapshot_id.is_none()
}

fn is_assigned_restore(restore: &ProteusRestore, node_name: &str) -> bool {
    let Some(status) = restore.status.as_ref() else {
        return false;
    };
    status.data_plane.as_ref() == Some(&DataPlane::Agent)
        && status.assigned_node.as_deref() == Some(node_name)
        && status.phase.as_ref() == Some(&RestorePhase::Running)
        && status.completed_at.is_none()
}

async fn run_backup_mover_pod(
    client: &Client,
    ns: &str,
    backup: &ProteusBackup,
    image: &str,
) -> Result<()> {
    let recipe = crate::backup::recipe::load_recipe(client, backup, ns)
        .await
        .map_err(anyhow::Error::msg)?;
    let name = backup.name_any();
    let pod_name = format!("proteus-bmv-{}", uuid_short());

    let (volumes, mounts) = pvc_mounts(&recipe.pvc_names, true);
    let pod = build_mover_pod(
        &pod_name,
        image,
        &recipe.target_namespace,
        PURPOSE_BACKUP_MOVER,
        ns,
        &name,
        backup.status.as_ref().and_then(|s| s.assigned_node.clone()),
        volumes,
        mounts,
        vec![
            "mover".into(),
            "backup".into(),
            "--namespace".into(),
            ns.into(),
            "--name".into(),
            name.clone(),
        ],
    );
    create_and_wait_mover(client, &recipe.target_namespace, pod, &pod_name).await
}

async fn run_restore_mover_pod(
    client: &Client,
    ns: &str,
    restore: &ProteusRestore,
    image: &str,
) -> Result<()> {
    let backup_ns = restore.spec.backup_namespace.as_deref().unwrap_or(ns);
    let backups: Api<ProteusBackup> = Api::namespaced(client.clone(), backup_ns);
    let backup = backups
        .get(&restore.spec.backup_ref)
        .await
        .context("load source backup")?;
    let recipe = crate::backup::recipe::load_recipe(client, &backup, backup_ns)
        .await
        .map_err(anyhow::Error::msg)?;
    let name = restore.name_any();
    let pod_name = format!("proteus-rmv-{}", uuid_short());

    let (volumes, mounts) = pvc_mounts(&recipe.pvc_names, false);
    let pod = build_mover_pod(
        &pod_name,
        image,
        &restore.spec.target_namespace,
        PURPOSE_RESTORE_MOVER,
        ns,
        &name,
        restore.status.as_ref().and_then(|s| s.assigned_node.clone()),
        volumes,
        mounts,
        vec![
            "mover".into(),
            "restore".into(),
            "--namespace".into(),
            ns.into(),
            "--name".into(),
            name.clone(),
        ],
    );
    create_and_wait_mover(client, &restore.spec.target_namespace, pod, &pod_name).await
}

fn pvc_mounts(pvc_names: &[String], read_only: bool) -> (Vec<Volume>, Vec<VolumeMount>) {
    let mut volumes = Vec::new();
    let mut mounts = Vec::new();
    for pvc in pvc_names {
        let vol_name = format!("pvc-{}", sanitize(pvc));
        volumes.push(Volume {
            name: vol_name.clone(),
            persistent_volume_claim: Some(PersistentVolumeClaimVolumeSource {
                claim_name: pvc.clone(),
                read_only: Some(read_only),
            }),
            ..Default::default()
        });
        mounts.push(VolumeMount {
            name: vol_name,
            mount_path: format!("{VOLUME_ROOT}/{pvc}"),
            read_only: Some(read_only),
            ..Default::default()
        });
    }
    (volumes, mounts)
}

#[allow(clippy::too_many_arguments)]
fn build_mover_pod(
    pod_name: &str,
    image: &str,
    namespace: &str,
    purpose: &str,
    owner_ns: &str,
    owner_name: &str,
    node_name: Option<String>,
    volumes: Vec<Volume>,
    mounts: Vec<VolumeMount>,
    args: Vec<String>,
) -> Pod {
    let mut labels = BTreeMap::new();
    labels.insert(LABEL_PURPOSE.to_string(), purpose.to_string());
    labels.insert(LABEL_OWNER.to_string(), owner_name.to_string());
    labels.insert(LABEL_OWNER_NS.to_string(), owner_ns.to_string());
    labels.insert("app.kubernetes.io/name".into(), "proteus".into());
    labels.insert("app.kubernetes.io/component".into(), "mover".into());

    Pod {
        metadata: ObjectMeta {
            name: Some(pod_name.to_string()),
            namespace: Some(namespace.to_string()),
            labels: Some(labels),
            ..Default::default()
        },
        spec: Some(PodSpec {
            service_account_name: Some(MOVER_SA.into()),
            restart_policy: Some("Never".into()),
            node_name,
            active_deadline_seconds: Some(MOVER_POD_TIMEOUT.as_secs() as i64),
            containers: vec![Container {
                name: "mover".into(),
                image: Some(image.to_string()),
                image_pull_policy: Some("IfNotPresent".into()),
                args: Some(args),
                env: Some(vec![EnvVar {
                    name: "RUST_LOG".into(),
                    value: Some("info,proteus_controller=debug".into()),
                    ..Default::default()
                }]),
                volume_mounts: Some(mounts),
                ..Default::default()
            }],
            volumes: Some(volumes),
            tolerations: Some(vec![Toleration {
                operator: Some("Exists".into()),
                ..Default::default()
            }]),
            ..Default::default()
        }),
        ..Default::default()
    }
}

async fn create_and_wait_mover(
    client: &Client,
    namespace: &str,
    pod: Pod,
    pod_name: &str,
) -> Result<()> {
    let pods: Api<Pod> = Api::namespaced(client.clone(), namespace);
    if mover_already_active(&pods, pod.metadata.labels.as_ref()).await? {
        info!(%pod_name, "mover already active; waiting on existing pod");
        // Fall through to wait by listing
        if let Some(existing) = existing_mover_name(&pods, pod.metadata.labels.as_ref()).await? {
            return wait_mover(&pods, &existing).await;
        }
    }

    pods.create(&PostParams::default(), &pod)
        .await
        .with_context(|| format!("create mover pod {pod_name}"))?;
    info!(%pod_name, %namespace, "created mover pod");
    wait_mover(&pods, pod_name).await
}

async fn wait_mover(pods: &Api<Pod>, pod_name: &str) -> Result<()> {
    let deadline = tokio::time::Instant::now() + MOVER_POD_TIMEOUT;
    loop {
        if tokio::time::Instant::now() > deadline {
            let _ = pods.delete(pod_name, &DeleteParams::default()).await;
            bail!("mover pod {pod_name} timed out");
        }
        let current = pods.get(pod_name).await.context("get mover pod")?;
        let phase = current
            .status
            .as_ref()
            .and_then(|s| s.phase.as_deref())
            .unwrap_or("");
        match phase {
            "Succeeded" => {
                info!(%pod_name, "mover pod succeeded");
                let _ = pods.delete(pod_name, &DeleteParams::default()).await;
                return Ok(());
            }
            "Failed" => {
                let _ = pods.delete(pod_name, &DeleteParams::default()).await;
                bail!("mover pod {pod_name} failed");
            }
            _ => tokio::time::sleep(Duration::from_secs(2)).await,
        }
    }
}

async fn mover_already_active(
    pods: &Api<Pod>,
    labels: Option<&BTreeMap<String, String>>,
) -> Result<bool> {
    Ok(existing_mover_name(pods, labels).await?.is_some())
}

async fn existing_mover_name(
    pods: &Api<Pod>,
    labels: Option<&BTreeMap<String, String>>,
) -> Result<Option<String>> {
    let Some(labels) = labels else {
        return Ok(None);
    };
    let purpose = labels.get(LABEL_PURPOSE).map(String::as_str).unwrap_or("");
    let owner = labels.get(LABEL_OWNER).map(String::as_str).unwrap_or("");
    let owner_ns = labels.get(LABEL_OWNER_NS).map(String::as_str).unwrap_or("");
    if purpose.is_empty() || owner.is_empty() {
        return Ok(None);
    }
    let lp = ListParams::default().labels(&format!(
        "{LABEL_PURPOSE}={purpose},{LABEL_OWNER}={owner},{LABEL_OWNER_NS}={owner_ns}"
    ));
    let list = pods.list(&lp).await.context("list mover pods")?;
    Ok(list.items.into_iter().find_map(|p| {
        let phase = p.status.as_ref().and_then(|s| s.phase.as_deref());
        if matches!(phase, Some("Pending" | "Running")) {
            p.metadata.name
        } else {
            None
        }
    }))
}

async fn fail_backup(client: &Client, ns: &str, name: &str, message: &str) -> Result<()> {
    let api: Api<ProteusBackup> = Api::namespaced(client.clone(), ns);
    let patch = serde_json::json!({
        "status": {
            "phase": "Failed",
            "message": message,
            "dataPlane": "agent",
            "lastFailureAt": chrono::Utc::now().to_rfc3339()
        }
    });
    api.patch_status(name, &PatchParams::default(), &Patch::Merge(&patch))
        .await?;
    Ok(())
}

async fn fail_restore(client: &Client, ns: &str, name: &str, message: &str) -> Result<()> {
    let api: Api<ProteusRestore> = Api::namespaced(client.clone(), ns);
    let patch = serde_json::json!({
        "status": {
            "phase": "Failed",
            "message": message,
            "dataPlane": "agent",
            "completedAt": chrono::Utc::now().to_rfc3339()
        }
    });
    api.patch_status(name, &PatchParams::default(), &Patch::Merge(&patch))
        .await?;
    Ok(())
}

fn sanitize(name: &str) -> String {
    name.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

fn uuid_short() -> String {
    uuid::Uuid::new_v4().to_string()[..8].to_string()
}

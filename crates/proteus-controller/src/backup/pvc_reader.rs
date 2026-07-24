use std::collections::BTreeMap;
use std::time::Duration;

use k8s_openapi::api::core::v1::{
    Container, PersistentVolumeClaimVolumeSource, Pod, PodSpec, Volume, VolumeMount,
};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::{ObjectMeta, OwnerReference};
use kube::api::{AttachParams, DeleteParams, ListParams, PostParams};
use kube::{Api, Client};
use proteus_core::backup::{ingest_volume_stream, VolumeSnapshot};
use proteus_core::crypto::EncryptionKey;
use proteus_core::ObjectStore;
use tokio::io::AsyncReadExt;
use tracing::{info, warn};
use uuid::Uuid;

const MOUNT_IMAGE: &str = "busybox:1.36";
const MOUNT_CONTAINER: &str = "mount";
const MOUNT_PATH: &str = "/data";
const POD_READY_TIMEOUT: Duration = Duration::from_secs(90);
const POD_READY_POLL_INTERVAL: Duration = Duration::from_secs(2);
/// Hard cap so a leaked mount Pod cannot sit forever after a controller crash.
const POD_ACTIVE_DEADLINE_SECS: i64 = 6 * 60 * 60;

const LABEL_PURPOSE: &str = "proteus.io/purpose";
const LABEL_BACKUP: &str = "proteus.io/backup";
const LABEL_BACKUP_NS: &str = "proteus.io/backup-namespace";
const PURPOSE_BACKUP_MOUNT: &str = "backup-mount";

/// Identity of the ProteusBackup that owns a mount Pod (for labels + same-ns ownerRef).
pub struct BackupMountOwner {
    pub backup_name: String,
    pub backup_namespace: String,
    pub backup_uid: String,
}

/// Stage updates while preparing the mount Pod / sizing the volume (before tar bytes flow).
#[derive(Clone, Copy, Debug)]
pub enum MountStage {
    CreatingPod,
    WaitingPodRunning,
    MeasuringSize,
    StreamingTar,
}

/// Delete leftover mount Pods for this backup (controller restart / cancelled runs).
pub async fn cleanup_backup_mount_pods(
    client: &Client,
    pvc_namespace: &str,
    backup_name: &str,
    backup_namespace: &str,
) -> Result<u32, String> {
    let pods: Api<Pod> = Api::namespaced(client.clone(), pvc_namespace);
    let lp = ListParams::default().labels(&format!(
        "{LABEL_PURPOSE}={PURPOSE_BACKUP_MOUNT},{LABEL_BACKUP}={backup_name},{LABEL_BACKUP_NS}={backup_namespace}"
    ));
    let list = pods
        .list(&lp)
        .await
        .map_err(|err| format!("failed to list mount pods: {err}"))?;

    let mut removed = 0u32;
    for pod in list.items {
        let Some(name) = pod.metadata.name.clone() else {
            continue;
        };
        match pods.delete(&name, &DeleteParams::default()).await {
            Ok(_) => {
                info!(%name, ns = %pvc_namespace, "deleted leftover backup mount pod");
                removed += 1;
            }
            Err(kube::Error::Api(err)) if err.code == 404 => {}
            Err(err) => warn!(%name, error = %err, "failed to delete leftover mount pod"),
        }
    }
    Ok(removed)
}

/// Mount `pvc_name` read-only, stream `tar` via kube exec into the CAS store (no full-archive buffer).
///
/// Callbacks may return `Err` to abort (e.g. backup CR deleted).
#[allow(clippy::too_many_arguments)]
pub async fn stream_pvc_into_store<Fs, FsFut, Fb, FbFut>(
    client: &Client,
    namespace: &str,
    pvc_name: &str,
    owner: &BackupMountOwner,
    store: &dyn ObjectStore,
    key: Option<&EncryptionKey>,
    mut on_stage: Fs,
    mut on_bytes: Fb,
) -> Result<VolumeSnapshot, String>
where
    Fs: FnMut(MountStage) -> FsFut,
    FsFut: std::future::Future<Output = Result<(), String>>,
    Fb: FnMut(u64, Option<u64>) -> FbFut,
    FbFut: std::future::Future<Output = Result<(), String>>,
{
    let pods: Api<Pod> = Api::namespaced(client.clone(), namespace);
    let pod_name = format!("proteus-bkp-{}", short_id());

    on_stage(MountStage::CreatingPod).await?;
    pods.create(
        &PostParams::default(),
        &build_mount_pod(&pod_name, pvc_name, owner, namespace),
    )
    .await
    .map_err(|err| format!("failed to create mount pod '{pod_name}': {err}"))?;

    let result = run_stream_backup(
        client,
        namespace,
        &pod_name,
        pvc_name,
        store,
        key,
        &mut on_stage,
        &mut on_bytes,
    )
    .await;

    if let Err(err) = pods.delete(&pod_name, &DeleteParams::default()).await {
        warn!(%pod_name, error = %err, "failed to delete mount pod (will leak until GC/cleanup)");
    }

    result
}

#[allow(clippy::too_many_arguments)]
async fn run_stream_backup<Fs, FsFut, Fb, FbFut>(
    client: &Client,
    namespace: &str,
    pod_name: &str,
    pvc_name: &str,
    store: &dyn ObjectStore,
    key: Option<&EncryptionKey>,
    on_stage: &mut Fs,
    on_bytes: &mut Fb,
) -> Result<VolumeSnapshot, String>
where
    Fs: FnMut(MountStage) -> FsFut,
    FsFut: std::future::Future<Output = Result<(), String>>,
    Fb: FnMut(u64, Option<u64>) -> FbFut,
    FbFut: std::future::Future<Output = Result<(), String>>,
{
    on_stage(MountStage::WaitingPodRunning).await?;
    wait_for_running(client, namespace, pod_name, on_stage).await?;
    on_stage(MountStage::MeasuringSize).await?;
    let total_hint = du_bytes(client, namespace, pod_name).await.ok();
    on_stage(MountStage::StreamingTar).await?;

    let pods: Api<Pod> = Api::namespaced(client.clone(), namespace);
    let ap = AttachParams {
        container: Some(MOUNT_CONTAINER.to_string()),
        stdin: false,
        stdout: true,
        stderr: false,
        tty: false,
        ..AttachParams::default()
    };

    let mut attached = pods
        .exec(
            pod_name,
            vec!["tar", "-cf", "-", "-C", MOUNT_PATH, "."],
            &ap,
        )
        .await
        .map_err(|err| format!("failed to exec tar in '{pod_name}': {err}"))?;

    let stdout = attached
        .stdout()
        .ok_or_else(|| "exec stream had no stdout".to_string())?;

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<u64>();
    let total_for_reporter = total_hint;
    let report = async {
        let mut last_done = 0u64;
        let mut last_reported = 0u64;
        while let Some(done) = rx.recv().await {
            last_done = done;
            if done.saturating_sub(last_reported) >= 1024 * 1024 {
                on_bytes(done, total_for_reporter).await?;
                last_reported = done;
            }
        }
        if last_done != last_reported {
            on_bytes(last_done, total_for_reporter).await?;
        }
        Ok::<(), String>(())
    };

    let ingest = async {
        let mut on_ingest = move |done: u64| {
            let _ = tx.send(done);
        };
        ingest_volume_stream(
            store,
            key,
            pvc_name,
            stdout,
            Some(&mut on_ingest as &mut (dyn FnMut(u64) + Send)),
        )
        .await
        .map_err(|err| format!("stream ingest failed: {err}"))
    };

    let (snap, report_result) = tokio::join!(ingest, report);
    report_result?;
    let snap = snap?;

    attached
        .join()
        .await
        .map_err(|err| format!("tar exec in '{pod_name}' failed: {err}"))?;

    Ok(snap)
}

fn short_id() -> String {
    Uuid::new_v4().simple().to_string()[..12].to_string()
}

fn build_mount_pod(
    pod_name: &str,
    pvc_name: &str,
    owner: &BackupMountOwner,
    pod_namespace: &str,
) -> Pod {
    let mut labels = BTreeMap::new();
    labels.insert(LABEL_PURPOSE.to_string(), PURPOSE_BACKUP_MOUNT.to_string());
    labels.insert(LABEL_BACKUP.to_string(), owner.backup_name.clone());
    labels.insert(LABEL_BACKUP_NS.to_string(), owner.backup_namespace.clone());

    // OwnerReference only works in the same namespace as the Backup CR.
    let owner_refs = if owner.backup_namespace == pod_namespace && !owner.backup_uid.is_empty() {
        Some(vec![OwnerReference {
            api_version: "proteus.io/v1alpha1".into(),
            kind: "ProteusBackup".into(),
            name: owner.backup_name.clone(),
            uid: owner.backup_uid.clone(),
            controller: Some(true),
            block_owner_deletion: Some(false),
        }])
    } else {
        None
    };

    Pod {
        metadata: ObjectMeta {
            name: Some(pod_name.to_string()),
            labels: Some(labels),
            owner_references: owner_refs,
            ..ObjectMeta::default()
        },
        spec: Some(PodSpec {
            restart_policy: Some("Never".to_string()),
            active_deadline_seconds: Some(POD_ACTIVE_DEADLINE_SECS),
            containers: vec![Container {
                name: MOUNT_CONTAINER.to_string(),
                image: Some(MOUNT_IMAGE.to_string()),
                command: Some(vec!["sleep".to_string(), "3600".to_string()]),
                volume_mounts: Some(vec![VolumeMount {
                    name: "data".to_string(),
                    mount_path: MOUNT_PATH.to_string(),
                    read_only: Some(true),
                    ..VolumeMount::default()
                }]),
                ..Container::default()
            }],
            volumes: Some(vec![Volume {
                name: "data".to_string(),
                persistent_volume_claim: Some(PersistentVolumeClaimVolumeSource {
                    claim_name: pvc_name.to_string(),
                    read_only: Some(true),
                }),
                ..Volume::default()
            }]),
            ..PodSpec::default()
        }),
        ..Pod::default()
    }
}

async fn wait_for_running<Fs, FsFut>(
    client: &Client,
    namespace: &str,
    pod_name: &str,
    on_stage: &mut Fs,
) -> Result<(), String>
where
    Fs: FnMut(MountStage) -> FsFut,
    FsFut: std::future::Future<Output = Result<(), String>>,
{
    let pods: Api<Pod> = Api::namespaced(client.clone(), namespace);
    let deadline = tokio::time::Instant::now() + POD_READY_TIMEOUT;

    loop {
        let pod = pods
            .get(pod_name)
            .await
            .map_err(|err| format!("failed to poll mount pod '{pod_name}': {err}"))?;
        let phase = pod.status.as_ref().and_then(|s| s.phase.as_deref());
        match phase {
            Some("Running") => return Ok(()),
            Some("Failed") => {
                return Err(format!("mount pod '{pod_name}' failed to start"));
            }
            _ => {
                if tokio::time::Instant::now() >= deadline {
                    return Err(format!(
                        "mount pod '{pod_name}' did not reach Running within {:?} (last phase={phase:?})",
                        POD_READY_TIMEOUT
                    ));
                }
                on_stage(MountStage::WaitingPodRunning).await?;
                tokio::time::sleep(POD_READY_POLL_INTERVAL).await;
            }
        }
    }
}

async fn du_bytes(client: &Client, namespace: &str, pod_name: &str) -> Result<u64, String> {
    let pods: Api<Pod> = Api::namespaced(client.clone(), namespace);
    let ap = AttachParams {
        container: Some(MOUNT_CONTAINER.to_string()),
        stdin: false,
        stdout: true,
        stderr: false,
        tty: false,
        ..AttachParams::default()
    };

    let mut attached = pods
        .exec(pod_name, vec!["du", "-sb", MOUNT_PATH], &ap)
        .await
        .map_err(|err| format!("failed to exec du in '{pod_name}': {err}"))?;

    let mut stdout = attached
        .stdout()
        .ok_or_else(|| "du exec stream had no stdout".to_string())?;
    let mut buf = Vec::new();
    stdout
        .read_to_end(&mut buf)
        .await
        .map_err(|err| format!("failed to read du output from '{pod_name}': {err}"))?;
    attached
        .join()
        .await
        .map_err(|err| format!("du exec in '{pod_name}' failed: {err}"))?;

    let text = String::from_utf8_lossy(&buf);
    let size = text
        .split_whitespace()
        .next()
        .and_then(|s| s.parse::<u64>().ok())
        .ok_or_else(|| format!("could not parse du output: {text:?}"))?;
    Ok(size)
}

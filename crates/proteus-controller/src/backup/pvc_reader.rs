use std::collections::BTreeMap;
use std::time::Duration;

use k8s_openapi::api::core::v1::{
    Container, PersistentVolumeClaimVolumeSource, Pod, PodSpec, Volume, VolumeMount,
};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
use kube::api::{AttachParams, DeleteParams, PostParams};
use kube::{Api, Client};
use tokio::io::AsyncReadExt;
use tracing::warn;
use uuid::Uuid;

const MOUNT_IMAGE: &str = "busybox:1.36";
const MOUNT_CONTAINER: &str = "mount";
const MOUNT_PATH: &str = "/data";
const POD_READY_TIMEOUT: Duration = Duration::from_secs(90);
const POD_READY_POLL_INTERVAL: Duration = Duration::from_secs(2);

/// Mount `pvc_name` read-only in a short-lived Pod, stream a tar of its contents via `exec`,
/// then delete the Pod. MVP: buffers the whole archive in memory.
pub async fn read_pvc_tar(
    client: &Client,
    namespace: &str,
    pvc_name: &str,
) -> Result<Vec<u8>, String> {
    let pods: Api<Pod> = Api::namespaced(client.clone(), namespace);
    let pod_name = format!("proteus-bkp-{}", short_id());

    pods.create(
        &PostParams::default(),
        &build_mount_pod(&pod_name, pvc_name),
    )
    .await
    .map_err(|err| format!("failed to create mount pod '{pod_name}': {err}"))?;

    let result = run_backup_exec(client, namespace, &pod_name).await;

    if let Err(err) = pods.delete(&pod_name, &DeleteParams::default()).await {
        warn!(%pod_name, error = %err, "failed to delete mount pod (will leak until manual cleanup)");
    }

    result
}

async fn run_backup_exec(
    client: &Client,
    namespace: &str,
    pod_name: &str,
) -> Result<Vec<u8>, String> {
    wait_for_running(client, namespace, pod_name).await?;
    exec_tar(client, namespace, pod_name).await
}

fn short_id() -> String {
    Uuid::new_v4().simple().to_string()[..12].to_string()
}

fn build_mount_pod(pod_name: &str, pvc_name: &str) -> Pod {
    let mut labels = BTreeMap::new();
    labels.insert("proteus.io/purpose".to_string(), "backup-mount".to_string());

    Pod {
        metadata: ObjectMeta {
            name: Some(pod_name.to_string()),
            labels: Some(labels),
            ..ObjectMeta::default()
        },
        spec: Some(PodSpec {
            restart_policy: Some("Never".to_string()),
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

async fn wait_for_running(client: &Client, namespace: &str, pod_name: &str) -> Result<(), String> {
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
                        "mount pod '{pod_name}' did not reach Running within {:?}",
                        POD_READY_TIMEOUT
                    ));
                }
                tokio::time::sleep(POD_READY_POLL_INTERVAL).await;
            }
        }
    }
}

async fn exec_tar(client: &Client, namespace: &str, pod_name: &str) -> Result<Vec<u8>, String> {
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

    let mut stdout = attached
        .stdout()
        .ok_or_else(|| "exec stream had no stdout".to_string())?;
    let mut buf = Vec::new();
    stdout
        .read_to_end(&mut buf)
        .await
        .map_err(|err| format!("failed to read tar stream from '{pod_name}': {err}"))?;

    attached
        .join()
        .await
        .map_err(|err| format!("tar exec in '{pod_name}' failed: {err}"))?;

    Ok(buf)
}

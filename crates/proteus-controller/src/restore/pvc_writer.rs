use std::collections::BTreeMap;
use std::time::Duration;

use k8s_openapi::api::core::v1::{
    Container, PersistentVolumeClaim, PersistentVolumeClaimVolumeSource, Pod, PodSpec, Volume,
    VolumeMount,
};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
use kube::api::{AttachParams, DeleteParams, PostParams};
use kube::{Api, Client};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tracing::warn;
use uuid::Uuid;

const MOUNT_IMAGE: &str = "busybox:1.36";
const MOUNT_CONTAINER: &str = "mount";
const MOUNT_PATH: &str = "/data";
const POD_READY_TIMEOUT: Duration = Duration::from_secs(90);
const POD_READY_POLL_INTERVAL: Duration = Duration::from_secs(2);

/// Mount `pvc_name` read-write in a short-lived Pod, optionally clear it, stream `tar_bytes` in
/// via `exec tar -xf -`, then delete the Pod. Mirrors the backup mount-pod pattern.
pub async fn write_pvc_tar(
    client: &Client,
    namespace: &str,
    pvc_name: &str,
    overwrite: bool,
    tar_bytes: Vec<u8>,
) -> Result<(), String> {
    ensure_pvc_exists(client, namespace, pvc_name).await?;

    let pods: Api<Pod> = Api::namespaced(client.clone(), namespace);
    let pod_name = format!("proteus-rst-{}", short_id());

    pods.create(
        &PostParams::default(),
        &build_mount_pod(&pod_name, pvc_name),
    )
    .await
    .map_err(|err| format!("failed to create mount pod '{pod_name}': {err}"))?;

    let result = run_restore_exec(client, namespace, &pod_name, overwrite, tar_bytes).await;

    if let Err(err) = pods.delete(&pod_name, &DeleteParams::default()).await {
        warn!(%pod_name, error = %err, "failed to delete mount pod (will leak until manual cleanup)");
    }

    result
}

async fn run_restore_exec(
    client: &Client,
    namespace: &str,
    pod_name: &str,
    overwrite: bool,
    tar_bytes: Vec<u8>,
) -> Result<(), String> {
    wait_for_running(client, namespace, pod_name).await?;

    if overwrite {
        clear_target(client, namespace, pod_name).await?;
    } else if !target_is_empty(client, namespace, pod_name).await? {
        return Err(format!(
            "target PVC already has data at {MOUNT_PATH} (set overwrite: true to replace it)"
        ));
    }

    exec_untar(client, namespace, pod_name, tar_bytes).await
}

async fn ensure_pvc_exists(client: &Client, namespace: &str, pvc_name: &str) -> Result<(), String> {
    let pvcs: Api<PersistentVolumeClaim> = Api::namespaced(client.clone(), namespace);
    match pvcs.get(pvc_name).await {
        Ok(_) => Ok(()),
        Err(kube::Error::Api(err)) if err.code == 404 => Err(format!(
            "target PVC '{pvc_name}' not found in namespace '{namespace}' \
             (create an empty PVC with that name before restoring)"
        )),
        Err(err) => Err(format!(
            "failed to look up target PVC '{pvc_name}' in namespace '{namespace}': {err}"
        )),
    }
}

fn short_id() -> String {
    Uuid::new_v4().simple().to_string()[..12].to_string()
}

fn build_mount_pod(pod_name: &str, pvc_name: &str) -> Pod {
    let mut labels = BTreeMap::new();
    labels.insert(
        "proteus.io/purpose".to_string(),
        "restore-mount".to_string(),
    );

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
                    ..VolumeMount::default()
                }]),
                ..Container::default()
            }],
            volumes: Some(vec![Volume {
                name: "data".to_string(),
                persistent_volume_claim: Some(PersistentVolumeClaimVolumeSource {
                    claim_name: pvc_name.to_string(),
                    read_only: Some(false),
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

/// `true` when the mount path has no entries — i.e. it is safe to restore into without
/// `overwrite`.
async fn target_is_empty(client: &Client, namespace: &str, pod_name: &str) -> Result<bool, String> {
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
            vec![
                "sh",
                "-c",
                &format!("find {MOUNT_PATH} -mindepth 1 -print -quit"),
            ],
            &ap,
        )
        .await
        .map_err(|err| format!("failed to check target PVC contents in '{pod_name}': {err}"))?;

    let mut stdout = attached
        .stdout()
        .ok_or_else(|| "exec stream had no stdout".to_string())?;
    let mut buf = Vec::new();
    stdout
        .read_to_end(&mut buf)
        .await
        .map_err(|err| format!("failed to read emptiness check from '{pod_name}': {err}"))?;

    attached
        .join()
        .await
        .map_err(|err| format!("emptiness check in '{pod_name}' failed: {err}"))?;

    Ok(buf.iter().all(u8::is_ascii_whitespace))
}

async fn clear_target(client: &Client, namespace: &str, pod_name: &str) -> Result<(), String> {
    let pods: Api<Pod> = Api::namespaced(client.clone(), namespace);
    // kube AttachParams requires at least one of stdin/stdout/stderr.
    let ap = AttachParams {
        container: Some(MOUNT_CONTAINER.to_string()),
        stdin: false,
        stdout: true,
        stderr: true,
        tty: false,
        ..AttachParams::default()
    };

    let mut attached = pods
        .exec(
            pod_name,
            vec![
                "sh",
                "-c",
                &format!(
                    "rm -rf {MOUNT_PATH}/.??* {MOUNT_PATH}/.[!.]* {MOUNT_PATH}/* 2>/dev/null; true"
                ),
            ],
            &ap,
        )
        .await
        .map_err(|err| format!("failed to clear target PVC in '{pod_name}': {err}"))?;

    if let Some(mut stdout) = attached.stdout() {
        let mut buf = Vec::new();
        let _ = stdout.read_to_end(&mut buf).await;
    }
    if let Some(mut stderr) = attached.stderr() {
        let mut buf = Vec::new();
        let _ = stderr.read_to_end(&mut buf).await;
    }

    attached
        .join()
        .await
        .map_err(|err| format!("clearing target PVC in '{pod_name}' failed: {err}"))
}

async fn exec_untar(
    client: &Client,
    namespace: &str,
    pod_name: &str,
    tar_bytes: Vec<u8>,
) -> Result<(), String> {
    let pods: Api<Pod> = Api::namespaced(client.clone(), namespace);
    let ap = AttachParams {
        container: Some(MOUNT_CONTAINER.to_string()),
        stdin: true,
        stdout: false,
        stderr: false,
        tty: false,
        ..AttachParams::default()
    };

    let mut attached = pods
        .exec(pod_name, vec!["tar", "-xf", "-", "-C", MOUNT_PATH], &ap)
        .await
        .map_err(|err| format!("failed to exec tar restore in '{pod_name}': {err}"))?;

    let mut stdin = attached
        .stdin()
        .ok_or_else(|| "exec stream had no stdin".to_string())?;
    stdin
        .write_all(&tar_bytes)
        .await
        .map_err(|err| format!("failed to write tar stream to '{pod_name}': {err}"))?;
    stdin
        .shutdown()
        .await
        .map_err(|err| format!("failed to close tar stream to '{pod_name}': {err}"))?;
    drop(stdin);

    attached
        .join()
        .await
        .map_err(|err| format!("tar restore exec in '{pod_name}' failed: {err}"))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_mount_pod_mounts_pvc_read_write() {
        let pod = build_mount_pod("proteus-rst-test", "target-pvc");
        let spec = pod.spec.expect("spec");
        let volume = spec.volumes.expect("volumes")[0].clone();
        let claim = volume.persistent_volume_claim.expect("pvc source");
        assert_eq!(claim.claim_name, "target-pvc");
        assert_eq!(claim.read_only, Some(false));
    }
}

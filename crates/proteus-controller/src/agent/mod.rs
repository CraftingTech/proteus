//! Node-agent process mode (`proteus-controller agent`).

mod identity;
mod mover;
mod work;

use std::time::Duration;

use anyhow::{bail, Context, Result};
use k8s_openapi::api::core::v1::Pod;
use kube::api::{Patch, PatchParams};
use kube::{Api, Client, ResourceExt};
use serde_json::json;
use tracing::{info, warn};

/// Label set on the agent Pod when it is ready to accept work on its node.
pub const AGENT_READY_LABEL: &str = "proteus.io/agent-ready";

/// Env var injected by the DaemonSet (`spec.nodeName`).
pub const NODE_NAME_ENV: &str = "NODE_NAME";

/// Env var for this Pod's name (downward API).
pub const POD_NAME_ENV: &str = "POD_NAME";

/// Env var for this Pod's namespace (downward API).
pub const POD_NAMESPACE_ENV: &str = "POD_NAMESPACE";

pub use identity::ensure_mover_identity;
pub use mover::run as run_mover;

pub async fn run() -> Result<()> {
    let node_name = std::env::var(NODE_NAME_ENV).unwrap_or_default();
    if node_name.is_empty() {
        bail!("{NODE_NAME_ENV} is required for agent mode (DaemonSet downward API)");
    }

    let pod_name = std::env::var(POD_NAME_ENV).unwrap_or_default();
    let pod_namespace =
        std::env::var(POD_NAMESPACE_ENV).unwrap_or_else(|_| "proteus-system".into());

    let client = Client::try_default()
        .await
        .context("failed to build Kubernetes client for agent")?;

    let mover_image = resolve_mover_image(&client, &pod_namespace, &pod_name).await;

    info!(
        version = env!("CARGO_PKG_VERSION"),
        %node_name,
        %pod_namespace,
        pod = %pod_name,
        image = %mover_image,
        "starting proteus-node-agent"
    );

    if !pod_name.is_empty() {
        if let Err(err) = mark_agent_ready(&client, &pod_namespace, &pod_name).await {
            warn!(error = %err, "failed to set agent Ready label (will retry in heartbeat)");
        }
    } else {
        warn!("{POD_NAME_ENV} unset; Ready label not applied");
    }

    let heartbeat = {
        let client = client.clone();
        let pod_namespace = pod_namespace.clone();
        let pod_name = pod_name.clone();
        async move {
            let mut ticker = tokio::time::interval(Duration::from_secs(30));
            loop {
                ticker.tick().await;
                if pod_name.is_empty() {
                    continue;
                }
                if let Err(err) = mark_agent_ready(&client, &pod_namespace, &pod_name).await {
                    warn!(error = %err, "agent Ready heartbeat failed");
                }
            }
        }
    };

    tokio::select! {
        result = work::run_work_loop(client, node_name, mover_image) => result,
        () = heartbeat => bail!("agent heartbeat terminated"),
    }
}

async fn resolve_mover_image(client: &Client, namespace: &str, pod_name: &str) -> String {
    if let Ok(image) = std::env::var("PROTEUS_IMAGE") {
        if !image.is_empty() {
            return image;
        }
    }
    if !pod_name.is_empty() {
        if let Ok(image) = own_container_image(client, namespace, pod_name).await {
            return image;
        }
    }
    "ghcr.io/craftingtech/proteus-controller:main".into()
}

async fn mark_agent_ready(client: &Client, namespace: &str, name: &str) -> Result<()> {
    let api: Api<Pod> = Api::namespaced(client.clone(), namespace);
    let patch = json!({
        "metadata": {
            "labels": {
                AGENT_READY_LABEL: "true"
            }
        }
    });
    api.patch(name, &PatchParams::default(), &Patch::Merge(&patch))
        .await
        .with_context(|| format!("patch Pod {namespace}/{name} Ready label"))?;
    let pod = api
        .get(name)
        .await
        .context("get agent Pod after Ready patch")?;
    info!(
        pod = %pod.name_any(),
        node = ?pod.spec.as_ref().and_then(|s| s.node_name.as_deref()),
        "agent Ready"
    );
    Ok(())
}

async fn own_container_image(client: &Client, namespace: &str, name: &str) -> Result<String> {
    let api: Api<Pod> = Api::namespaced(client.clone(), namespace);
    let pod = api.get(name).await.context("get own agent Pod")?;
    let image = pod
        .spec
        .as_ref()
        .and_then(|s| s.containers.first())
        .and_then(|c| c.image.clone())
        .filter(|s| !s.is_empty())
        .context("agent Pod has no container image")?;
    Ok(image)
}

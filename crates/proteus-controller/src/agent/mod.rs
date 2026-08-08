//! Node-agent process mode (`proteus-controller agent`).
//!
//! Phase 1: connect, resolve `NODE_NAME`, mark Ready, heartbeat.
//! Later phases watch assigned Backup/Restore CRs and spawn movers.

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

pub async fn run() -> Result<()> {
    let node_name = std::env::var(NODE_NAME_ENV).unwrap_or_default();
    if node_name.is_empty() {
        bail!("{NODE_NAME_ENV} is required for agent mode (DaemonSet downward API)");
    }

    let pod_name = std::env::var(POD_NAME_ENV).unwrap_or_default();
    let pod_namespace = std::env::var(POD_NAMESPACE_ENV).unwrap_or_else(|_| "proteus-system".into());

    let client = Client::try_default()
        .await
        .context("failed to build Kubernetes client for agent")?;

    info!(
        version = env!("CARGO_PKG_VERSION"),
        %node_name,
        %pod_namespace,
        pod = %pod_name,
        "starting proteus-node-agent"
    );

    if !pod_name.is_empty() {
        if let Err(err) = mark_agent_ready(&client, &pod_namespace, &pod_name).await {
            warn!(error = %err, "failed to set agent Ready label (will retry in heartbeat)");
        }
    } else {
        warn!("{POD_NAME_ENV} unset; Ready label not applied");
    }

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
    let pod = api.get(name).await.context("get agent Pod after Ready patch")?;
    info!(
        pod = %pod.name_any(),
        node = ?pod.spec.as_ref().and_then(|s| s.node_name.as_deref()),
        "agent Ready"
    );
    Ok(())
}

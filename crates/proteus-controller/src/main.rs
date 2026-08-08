mod agent;
mod backup;
mod controllers;
mod data_plane;
mod error;
mod restore;

use std::net::SocketAddr;
use std::time::Duration;

use anyhow::{Context, Result};
use proteus_api::{router, serve, ApiState, ClusterSnapshot};
use tracing::{info, level_filters::LevelFilter, warn};
use tracing_subscriber::EnvFilter;

use crate::controllers::ControllerSet;

#[tokio::main]
async fn main() -> Result<()> {
    init_tracing()?;

    let mode = std::env::args().nth(1);
    match mode.as_deref() {
        Some("agent") => crate::agent::run().await,
        Some("mover") => {
            anyhow::bail!("mover subcommand is not implemented yet (phase 3/4 of node-agent)")
        }
        Some(other) => {
            anyhow::bail!("unknown mode {other:?}; expected `agent`, `mover`, or no argument")
        }
        None => run_controller().await,
    }
}

async fn run_controller() -> Result<()> {
    let client = kube::Client::try_default()
        .await
        .context("failed to build Kubernetes client (check KUBECONFIG / ~/.kube/config)")?;

    let api_state = ApiState::new(
        client.clone(),
        ClusterSnapshot {
            version: env!("CARGO_PKG_VERSION").to_string(),
            repositories: 0,
            backups: 0,
            restores: 0,
            last_reconcile_at: None,
        },
    );
    api_state.refresh_readiness().await;
    if let Err(err) = api_state.refresh_counts().await {
        warn!(error = %err, "initial CR count refresh failed");
    }

    let controllers = ControllerSet::new(client, api_state.clone());

    let api_addr: SocketAddr = match std::env::var("PROTEUS_API_ADDR") {
        Ok(value) => value,
        Err(_) => "0.0.0.0:8080".to_string(),
    }
    .parse()
    .context("invalid PROTEUS_API_ADDR")?;

    let readiness_poll = {
        let state = api_state.clone();
        async move {
            loop {
                tokio::time::sleep(Duration::from_secs(15)).await;
                state.refresh_readiness().await;
            }
        }
    };

    let api = serve(api_addr, router(api_state));
    let reconcile = controllers.run();

    info!(
        version = env!("CARGO_PKG_VERSION"),
        %api_addr,
        "starting proteus-controller"
    );

    tokio::select! {
        result = api => result.context("API server exited")?,
        result = reconcile => result.context("controller set exited")?,
        () = readiness_poll => anyhow::bail!("readiness poll terminated"),
    }

    Ok(())
}

fn init_tracing() -> Result<()> {
    let filter = EnvFilter::builder()
        .with_default_directive(LevelFilter::INFO.into())
        .from_env_lossy();

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(true)
        .try_init()
        .map_err(|e| anyhow::anyhow!("tracing init failed: {e}"))?;
    Ok(())
}

mod controllers;
mod error;

use std::net::SocketAddr;

use anyhow::{bail, Context, Result};
use k8s_openapi::apiextensions_apiserver::pkg::apis::apiextensions::v1::CustomResourceDefinition;
use kube::{Api, Client};
use proteus_api::{router, serve, ApiState, ClusterSnapshot};
use tracing::{info, warn, level_filters::LevelFilter};
use tracing_subscriber::EnvFilter;

use crate::controllers::ControllerSet;

const REQUIRED_CRDS: &[&str] = &[
    "proteusrepositories.proteus.io",
    "proteusbackups.proteus.io",
    "proteusrestores.proteus.io",
];

#[tokio::main]
async fn main() -> Result<()> {
    init_tracing()?;

    let client = kube::Client::try_default()
        .await
        .context("failed to build Kubernetes client (check KUBECONFIG / ~/.kube/config)")?;

    ensure_crds(&client).await?;

    let api_state = ApiState::new(ClusterSnapshot {
        version: env!("CARGO_PKG_VERSION").to_string(),
        repositories: 0,
        backups: 0,
        restores: 0,
        last_reconcile_at: None,
    });
    let controllers = ControllerSet::new(client, api_state.clone());

    let api_addr: SocketAddr = match std::env::var("PROTEUS_API_ADDR") {
        Ok(value) => value,
        Err(_) => "0.0.0.0:8080".to_string(),
    }
    .parse()
    .context("invalid PROTEUS_API_ADDR")?;

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
    }

    Ok(())
}

async fn ensure_crds(client: &Client) -> Result<()> {
    let crds: Api<CustomResourceDefinition> = Api::all(client.clone());
    let mut missing = Vec::new();

    for name in REQUIRED_CRDS {
        match crds.get(name).await {
            Ok(_) => info!(crd = %name, "CRD present"),
            Err(kube::Error::Api(err)) if err.code == 404 => missing.push(*name),
            Err(err) => {
                return Err(err).with_context(|| format!("failed to check CRD {name}"));
            }
        }
    }

    if missing.is_empty() {
        return Ok(());
    }

    warn!(?missing, "required CRDs missing from the cluster");
    bail!(
        "missing CRDs: {}\n\
         Install them with: just ensure-crds\n\
         (or: kubectl apply -k deploy/kustomize/crds)",
        missing.join(", ")
    );
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

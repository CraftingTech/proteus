use std::sync::Arc;

use chrono::Utc;
use k8s_openapi::apiextensions_apiserver::pkg::apis::apiextensions::v1::CustomResourceDefinition;
use kube::{Api, Client, ResourceExt};
use parking_lot::RwLock;
use proteus_crd::{ProteusBackup, ProteusRepository, ProteusRestore};
use serde::Serialize;
use tracing::{info, warn};

pub const REQUIRED_CRDS: &[&str] = &[
    "proteusrepositories.proteus.io",
    "proteusbackups.proteus.io",
    "proteusrestores.proteus.io",
];

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClusterSnapshot {
    pub version: String,
    pub repositories: u64,
    pub backups: u64,
    pub restores: u64,
    pub last_reconcile_at: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct Readiness {
    pub kube_reachable: bool,
    pub crds_ready: bool,
}

impl Readiness {
    pub fn is_ready(&self) -> bool {
        self.kube_reachable && self.crds_ready
    }
}

#[derive(Clone)]
pub struct ApiState {
    pub snapshot: Arc<RwLock<ClusterSnapshot>>,
    pub client: Client,
    pub readiness: Arc<RwLock<Readiness>>,
}

impl ApiState {
    pub fn new(client: Client, snapshot: ClusterSnapshot) -> Self {
        Self {
            snapshot: Arc::new(RwLock::new(snapshot)),
            client,
            readiness: Arc::new(RwLock::new(Readiness::default())),
        }
    }

    pub fn is_ready(&self) -> bool {
        self.readiness.read().is_ready()
    }

    pub fn mark_reconciled(&self) {
        self.snapshot.write().last_reconcile_at = Some(Utc::now().to_rfc3339());
    }

    pub async fn refresh_readiness(&self) {
        match check_required_crds(&self.client).await {
            Ok(missing) => {
                let ready = missing.is_empty();
                let mut gate = self.readiness.write();
                gate.kube_reachable = true;
                gate.crds_ready = ready;
                if ready {
                    info!("readiness: kube reachable, required CRDs present");
                } else {
                    warn!(?missing, "readiness: required CRDs missing");
                }
            }
            Err(err) if is_auth_or_rbac_error(&err) => {
                warn!(error = %err, "readiness: CRD check denied by auth/RBAC");
                let mut gate = self.readiness.write();
                gate.kube_reachable = true;
                gate.crds_ready = false;
            }
            Err(err) => {
                warn!(error = %err, "readiness: kube unreachable");
                let mut gate = self.readiness.write();
                gate.kube_reachable = false;
                gate.crds_ready = false;
            }
        }
    }

    pub async fn refresh_counts(&self) -> Result<(), kube::Error> {
        let repos = Api::<ProteusRepository>::all(self.client.clone())
            .list(&Default::default())
            .await?
            .items
            .len() as u64;
        let backups = Api::<ProteusBackup>::all(self.client.clone())
            .list(&Default::default())
            .await?
            .items
            .len() as u64;
        let restores = Api::<ProteusRestore>::all(self.client.clone())
            .list(&Default::default())
            .await?
            .items
            .len() as u64;

        let mut snap = self.snapshot.write();
        snap.repositories = repos;
        snap.backups = backups;
        snap.restores = restores;
        snap.last_reconcile_at = Some(Utc::now().to_rfc3339());
        Ok(())
    }
}

fn is_auth_or_rbac_error(err: &kube::Error) -> bool {
    matches!(
        err,
        kube::Error::Api(api) if api.code == 401 || api.code == 403
    )
}

pub async fn check_required_crds(client: &Client) -> Result<Vec<&'static str>, kube::Error> {
    let crds: Api<CustomResourceDefinition> = Api::all(client.clone());
    let mut missing = Vec::new();

    for name in REQUIRED_CRDS {
        match crds.get(name).await {
            Ok(_) => {}
            Err(kube::Error::Api(err)) if err.code == 404 => missing.push(*name),
            Err(err) => return Err(err),
        }
    }

    Ok(missing)
}

pub fn object_namespace<K: ResourceExt>(obj: &K) -> String {
    obj.namespace().unwrap_or_else(|| "default".to_string())
}

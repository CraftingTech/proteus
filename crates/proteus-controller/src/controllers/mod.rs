mod backup;
mod backup_policy;
pub(crate) mod repository;
mod restore;

use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result};
use futures::StreamExt;
use kube::runtime::controller::{Action, Controller};
use kube::runtime::watcher::Config;
use kube::{Api, Client};
use proteus_api::ApiState;
use proteus_crd::{ProteusBackup, ProteusBackupPolicy, ProteusRepository, ProteusRestore};
use tracing::{error, info};

use self::backup::reconcile_backup;
use self::backup_policy::reconcile_backup_policy;
use self::repository::reconcile_repository;
use self::restore::reconcile_restore;
use crate::error::ControllerError;

pub struct ControllerSet {
    client: Client,
    api_state: ApiState,
}

impl ControllerSet {
    pub fn new(client: Client, api_state: ApiState) -> Self {
        Self { client, api_state }
    }

    pub async fn run(self) -> Result<()> {
        let client = self.client.clone();
        let ctx = Arc::new(ReconcileCtx {
            client: self.client,
            api_state: self.api_state,
            active_backups: Mutex::new(HashSet::new()),
        });

        let repos = Api::<ProteusRepository>::all(client.clone());
        let policies = Api::<ProteusBackupPolicy>::all(client.clone());
        let backups = Api::<ProteusBackup>::all(client.clone());
        let restores = Api::<ProteusRestore>::all(client);

        let repo_ctrl = Controller::new(repos, Config::default())
            .run(
                |obj, ctx| async move { reconcile_repository(obj, ctx).await },
                error_policy,
                ctx.clone(),
            )
            .for_each(|res| async move {
                match res {
                    Ok((obj_ref, _)) => info!(name = %obj_ref.name, "repository reconciled"),
                    Err(e) => error!(error = %e, "repository reconcile failed"),
                }
            });

        let policy_ctrl = Controller::new(policies, Config::default())
            .run(
                |obj, ctx| async move { reconcile_backup_policy(obj, ctx).await },
                error_policy,
                ctx.clone(),
            )
            .for_each(|res| async move {
                match res {
                    Ok((obj_ref, _)) => info!(name = %obj_ref.name, "backup policy reconciled"),
                    Err(e) => error!(error = %e, "backup policy reconcile failed"),
                }
            });

        let backup_ctrl = Controller::new(backups, Config::default())
            .run(
                |obj, ctx| async move { reconcile_backup(obj, ctx).await },
                error_policy,
                ctx.clone(),
            )
            .for_each(|res| async move {
                match res {
                    Ok((obj_ref, _)) => info!(name = %obj_ref.name, "backup reconciled"),
                    Err(e) => error!(error = %e, "backup reconcile failed"),
                }
            });

        let restore_ctrl = Controller::new(restores, Config::default())
            .run(
                |obj, ctx| async move { reconcile_restore(obj, ctx).await },
                error_policy,
                ctx,
            )
            .for_each(|res| async move {
                match res {
                    Ok((obj_ref, _)) => info!(name = %obj_ref.name, "restore reconciled"),
                    Err(e) => error!(error = %e, "restore reconcile failed"),
                }
            });

        tokio::select! {
            () = repo_ctrl => Err(anyhow::anyhow!("repository controller terminated")),
            () = policy_ctrl => Err(anyhow::anyhow!("backup policy controller terminated")),
            () = backup_ctrl => Err(anyhow::anyhow!("backup controller terminated")),
            () = restore_ctrl => Err(anyhow::anyhow!("restore controller terminated")),
        }
        .context("controller runtime stopped")
    }
}

pub struct ReconcileCtx {
    pub client: Client,
    pub api_state: ApiState,
    /// In-flight backup keys (`namespace/name`) so progress status patches do not start a second run.
    pub active_backups: Mutex<HashSet<String>>,
}

fn error_policy<K>(_obj: Arc<K>, error: &ControllerError, _ctx: Arc<ReconcileCtx>) -> Action {
    error!(error = %error, "reconcile error; requeue");
    Action::requeue(Duration::from_secs(30))
}

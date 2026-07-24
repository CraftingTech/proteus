use kube::api::ListParams;
use kube::{Api, ResourceExt};
use proteus_crd::{ProteusBackup, ProteusRepository, ProteusRestore, RepositoryBackend};
use serde::Serialize;
use serde_json::Value;

use crate::error::ApiResult;
use crate::state::{object_namespace, ApiState};

fn phase_label<T: Serialize>(phase: &T) -> Option<String> {
    match serde_json::to_value(phase) {
        Ok(Value::String(label)) => Some(label),
        _ => None,
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RepositoryListItem {
    pub name: String,
    pub namespace: String,
    pub phase: Option<String>,
    pub backend: Option<String>,
    pub message: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupListItem {
    pub name: String,
    pub namespace: String,
    pub repository_ref: String,
    pub target_namespace: String,
    pub schedule: Option<String>,
    pub phase: Option<String>,
    pub message: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RestoreListItem {
    pub name: String,
    pub namespace: String,
    pub backup_ref: String,
    pub target_namespace: String,
    pub phase: Option<String>,
    pub message: Option<String>,
}

pub async fn list_repositories(state: &ApiState) -> ApiResult<Vec<RepositoryListItem>> {
    let api: Api<ProteusRepository> = Api::all(state.client.clone());
    let list = api.list(&ListParams::default()).await?;
    Ok(list
        .items
        .into_iter()
        .map(|obj| {
            let backend = match &obj.spec.backend {
                RepositoryBackend::Local(_) => Some("local".to_string()),
                RepositoryBackend::S3(_) => Some("s3".to_string()),
            };
            RepositoryListItem {
                name: obj.name_any(),
                namespace: object_namespace(&obj),
                phase: obj
                    .status
                    .as_ref()
                    .and_then(|s| s.phase.as_ref().and_then(phase_label)),
                backend,
                message: obj.status.as_ref().and_then(|s| s.message.clone()),
            }
        })
        .collect())
}

pub async fn list_backups(state: &ApiState) -> ApiResult<Vec<BackupListItem>> {
    let api: Api<ProteusBackup> = Api::all(state.client.clone());
    let list = api.list(&ListParams::default()).await?;
    Ok(list
        .items
        .into_iter()
        .map(|obj| BackupListItem {
            name: obj.name_any(),
            namespace: object_namespace(&obj),
            repository_ref: obj.spec.repository_ref.clone(),
            target_namespace: obj.spec.target_namespace.clone(),
            schedule: obj.spec.schedule.clone(),
            phase: obj
                .status
                .as_ref()
                .and_then(|s| s.phase.as_ref().and_then(phase_label)),
            message: obj.status.as_ref().and_then(|s| s.message.clone()),
        })
        .collect())
}

pub async fn list_restores(state: &ApiState) -> ApiResult<Vec<RestoreListItem>> {
    let api: Api<ProteusRestore> = Api::all(state.client.clone());
    let list = api.list(&ListParams::default()).await?;
    Ok(list
        .items
        .into_iter()
        .map(|obj| RestoreListItem {
            name: obj.name_any(),
            namespace: object_namespace(&obj),
            backup_ref: obj.spec.backup_ref.clone(),
            target_namespace: obj.spec.target_namespace.clone(),
            phase: obj
                .status
                .as_ref()
                .and_then(|s| s.phase.as_ref().and_then(phase_label)),
            message: obj.status.as_ref().and_then(|s| s.message.clone()),
        })
        .collect())
}

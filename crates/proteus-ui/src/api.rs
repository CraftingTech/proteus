use serde::Deserialize;

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClusterSnapshot {
    pub version: String,
    pub repositories: u64,
    pub backups: u64,
    pub restores: u64,
    pub last_reconcile_at: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RepositoryListItem {
    pub name: String,
    pub namespace: String,
    pub phase: Option<String>,
    pub backend: Option<String>,
    pub message: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
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

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RestoreListItem {
    pub name: String,
    pub namespace: String,
    pub backup_ref: String,
    pub target_namespace: String,
    pub phase: Option<String>,
    pub message: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ApiClientError {
    pub message: String,
}

impl std::fmt::Display for ApiClientError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

async fn get_json<T: for<'de> Deserialize<'de>>(path: &str) -> Result<T, ApiClientError> {
    let response = gloo_net::http::Request::get(path)
        .send()
        .await
        .map_err(|err| ApiClientError {
            message: format!("request failed: {err}"),
        })?;

    let status = response.status();
    if !(200..300).contains(&status) {
        let body = response.text().await.unwrap_or_default();
        return Err(ApiClientError {
            message: format!("HTTP {status}: {body}"),
        });
    }

    response.json::<T>().await.map_err(|err| ApiClientError {
        message: format!("invalid JSON: {err}"),
    })
}

pub async fn get_cluster() -> Result<ClusterSnapshot, ApiClientError> {
    get_json("/api/v1/cluster").await
}

pub async fn list_repositories() -> Result<Vec<RepositoryListItem>, ApiClientError> {
    get_json("/api/v1/repositories").await
}

pub async fn list_backups() -> Result<Vec<BackupListItem>, ApiClientError> {
    get_json("/api/v1/backups").await
}

pub async fn list_restores() -> Result<Vec<RestoreListItem>, ApiClientError> {
    get_json("/api/v1/restores").await
}

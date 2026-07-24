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

/// Empty when embedded in the controller (`just run`). Set at compile time for `just ui`.
fn api_url(path: &str) -> String {
    const BASE: &str = match option_env!("PROTEUS_API_BASE") {
        Some(base) => base,
        None => "",
    };
    if BASE.is_empty() {
        path.to_string()
    } else {
        format!("{BASE}{path}")
    }
}

async fn get_json<T: for<'de> Deserialize<'de>>(path: &str) -> Result<T, ApiClientError> {
    let url = api_url(path);
    let response = gloo_net::http::Request::get(&url)
        .send()
        .await
        .map_err(|err| ApiClientError {
            message: format!("request failed: {err}"),
        })?;

    let status = response.status();
    let body = response.text().await.map_err(|err| ApiClientError {
        message: format!("failed to read body: {err}"),
    })?;

    if !(200..300).contains(&status) {
        return Err(ApiClientError {
            message: format!("HTTP {status}: {body}"),
        });
    }

    if body.trim_start().starts_with('<') {
        return Err(ApiClientError {
            message: "API returned HTML instead of JSON — start the controller (`just run`) \
                      or use `just ui` (port 5173) against an API on :8080"
                .into(),
        });
    }

    serde_json::from_str(&body).map_err(|err| ApiClientError {
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

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InventoryItem {
    pub kind: String,
    pub name: String,
    pub namespace: String,
    pub extra: Option<String>,
}

pub async fn get_inventory(
    namespace: Option<&str>,
    kind: Option<&str>,
    q: Option<&str>,
) -> Result<Vec<InventoryItem>, ApiClientError> {
    let mut params = Vec::new();
    if let Some(ns) = namespace.filter(|s| !s.is_empty()) {
        params.push(format!("namespace={}", urlencoding_lite(ns)));
    }
    if let Some(kind) = kind.filter(|s| !s.is_empty() && *s != "All") {
        params.push(format!("kind={}", urlencoding_lite(kind)));
    }
    if let Some(q) = q.filter(|s| !s.is_empty()) {
        params.push(format!("q={}", urlencoding_lite(q)));
    }
    let path = if params.is_empty() {
        "/api/v1/inventory".to_string()
    } else {
        format!("/api/v1/inventory?{}", params.join("&"))
    };
    get_json(&path).await
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NamespaceItem {
    pub name: String,
}

pub async fn list_namespaces() -> Result<Vec<NamespaceItem>, ApiClientError> {
    get_json("/api/v1/namespaces").await
}

fn urlencoding_lite(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for b in value.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

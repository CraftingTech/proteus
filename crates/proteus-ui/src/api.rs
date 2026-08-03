use serde::{Deserialize, Serialize};

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

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateRepositoryRequest {
    pub name: String,
    pub namespace: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub encryption_enabled: bool,
    pub backend: CreateRepositoryBackend,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", tag = "type")]
pub enum CreateRepositoryBackend {
    #[serde(rename = "local")]
    Local { path: String },
    #[serde(rename = "s3")]
    S3 {
        bucket: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        prefix: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        endpoint: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        region: Option<String>,
        /// Optional: use an existing Secret instead of pasting keys.
        #[serde(
            rename = "credentialsSecretRef",
            skip_serializing_if = "Option::is_none"
        )]
        credentials_secret_ref: Option<String>,
        #[serde(rename = "accessKeyId", skip_serializing_if = "Option::is_none")]
        access_key_id: Option<String>,
        #[serde(rename = "secretAccessKey", skip_serializing_if = "Option::is_none")]
        secret_access_key: Option<String>,
        #[serde(rename = "forcePathStyle")]
        force_path_style: bool,
    },
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupPolicyListItem {
    pub name: String,
    pub namespace: String,
    pub repository_ref: String,
    #[serde(default)]
    pub repository_namespace: Option<String>,
    pub target_namespace: String,
    #[serde(default)]
    pub pvc_names: Vec<String>,
    pub schedule: Option<String>,
    pub keep_last: u32,
    #[serde(default)]
    pub max_age_days: Option<u32>,
    pub phase: Option<String>,
    pub message: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateBackupPolicyRequest {
    pub name: String,
    pub namespace: String,
    pub repository_ref: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repository_namespace: Option<String>,
    pub target_namespace: String,
    pub pvc_names: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupListItem {
    pub name: String,
    pub namespace: String,
    #[serde(default)]
    pub policy_ref: Option<String>,
    pub repository_ref: String,
    pub target_namespace: String,
    #[serde(default)]
    pub pvc_names: Vec<String>,
    pub schedule: Option<String>,
    pub phase: Option<String>,
    pub message: Option<String>,
    #[serde(default)]
    pub last_snapshot_id: Option<String>,
    #[serde(default)]
    pub progress_percent: Option<u8>,
    #[serde(default)]
    pub duration_seconds: Option<u64>,
    #[serde(default)]
    pub throughput_bytes_per_sec: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateBackupRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub namespace: Option<String>,
    pub policy_ref: String,
    pub policy_namespace: String,
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
    #[serde(default)]
    pub restored_snapshot_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateRestoreRequest {
    pub name: String,
    pub namespace: String,
    pub backup_ref: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backup_namespace: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snapshot_id: Option<String>,
    pub target_namespace: String,
    pub overwrite: bool,
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

async fn read_error_body(response: gloo_net::http::Response) -> String {
    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    if let Ok(json) = serde_json::from_str::<serde_json::Value>(&body) {
        if let Some(err) = json.get("error").and_then(|v| v.as_str()) {
            return format!("HTTP {status}: {err}");
        }
    }
    if body.is_empty() {
        format!("HTTP {status}")
    } else {
        format!("HTTP {status}: {body}")
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
    if !(200..300).contains(&status) {
        return Err(ApiClientError {
            message: read_error_body(response).await,
        });
    }

    let body = response.text().await.map_err(|err| ApiClientError {
        message: format!("failed to read body: {err}"),
    })?;

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

async fn send_json<T: for<'de> Deserialize<'de>, B: Serialize>(
    method: &str,
    path: &str,
    body: &B,
) -> Result<T, ApiClientError> {
    let url = api_url(path);
    let payload = serde_json::to_string(body).map_err(|err| ApiClientError {
        message: format!("failed to serialize body: {err}"),
    })?;
    let request = match method {
        "POST" => gloo_net::http::Request::post(&url),
        "PATCH" => gloo_net::http::Request::patch(&url),
        _ => {
            return Err(ApiClientError {
                message: format!("unsupported method {method}"),
            })
        }
    };
    let response = request
        .header("Content-Type", "application/json")
        .body(payload)
        .map_err(|err| ApiClientError {
            message: format!("failed to build request: {err}"),
        })?
        .send()
        .await
        .map_err(|err| ApiClientError {
            message: format!("request failed: {err}"),
        })?;

    let status = response.status();
    if !(200..300).contains(&status) {
        return Err(ApiClientError {
            message: read_error_body(response).await,
        });
    }

    let body = response.text().await.map_err(|err| ApiClientError {
        message: format!("failed to read body: {err}"),
    })?;
    serde_json::from_str(&body).map_err(|err| ApiClientError {
        message: format!("invalid JSON: {err}"),
    })
}

async fn send_empty(method: &str, path: &str) -> Result<(), ApiClientError> {
    let url = api_url(path);
    let request = match method {
        "DELETE" => gloo_net::http::Request::delete(&url),
        _ => {
            return Err(ApiClientError {
                message: format!("unsupported method {method}"),
            })
        }
    };
    let response = request.send().await.map_err(|err| ApiClientError {
        message: format!("request failed: {err}"),
    })?;
    let status = response.status();
    if !(200..300).contains(&status) {
        return Err(ApiClientError {
            message: read_error_body(response).await,
        });
    }
    Ok(())
}

pub async fn get_cluster() -> Result<ClusterSnapshot, ApiClientError> {
    get_json("/api/v1/cluster").await
}

pub async fn list_repositories() -> Result<Vec<RepositoryListItem>, ApiClientError> {
    get_json("/api/v1/repositories").await
}

pub async fn create_repository(
    req: &CreateRepositoryRequest,
) -> Result<RepositoryListItem, ApiClientError> {
    send_json("POST", "/api/v1/repositories", req).await
}

pub async fn delete_repository(namespace: &str, name: &str) -> Result<(), ApiClientError> {
    let path = format!(
        "/api/v1/repositories/{}/{}",
        urlencoding_lite(namespace),
        urlencoding_lite(name)
    );
    send_empty("DELETE", &path).await
}

pub async fn list_backup_policies() -> Result<Vec<BackupPolicyListItem>, ApiClientError> {
    get_json("/api/v1/backup-policies").await
}

pub async fn create_backup_policy(
    req: &CreateBackupPolicyRequest,
) -> Result<BackupPolicyListItem, ApiClientError> {
    send_json("POST", "/api/v1/backup-policies", req).await
}

pub async fn delete_backup_policy(namespace: &str, name: &str) -> Result<(), ApiClientError> {
    let path = format!(
        "/api/v1/backup-policies/{}/{}",
        urlencoding_lite(namespace),
        urlencoding_lite(name)
    );
    send_empty("DELETE", &path).await
}

pub async fn list_backups() -> Result<Vec<BackupListItem>, ApiClientError> {
    get_json("/api/v1/backups").await
}

pub async fn create_backup(req: &CreateBackupRequest) -> Result<BackupListItem, ApiClientError> {
    send_json("POST", "/api/v1/backups", req).await
}

pub async fn delete_backup(namespace: &str, name: &str) -> Result<(), ApiClientError> {
    let path = format!(
        "/api/v1/backups/{}/{}",
        urlencoding_lite(namespace),
        urlencoding_lite(name)
    );
    send_empty("DELETE", &path).await
}

pub async fn list_restores() -> Result<Vec<RestoreListItem>, ApiClientError> {
    get_json("/api/v1/restores").await
}

pub async fn create_restore(req: &CreateRestoreRequest) -> Result<RestoreListItem, ApiClientError> {
    send_json("POST", "/api/v1/restores", req).await
}

pub async fn delete_restore(namespace: &str, name: &str) -> Result<(), ApiClientError> {
    let path = format!(
        "/api/v1/restores/{}/{}",
        urlencoding_lite(namespace),
        urlencoding_lite(name)
    );
    send_empty("DELETE", &path).await
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

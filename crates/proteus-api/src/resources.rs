use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use k8s_openapi::api::core::v1::Secret;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::{ObjectMeta, OwnerReference};
use k8s_openapi::ByteString;
use kube::api::{DeleteParams, ListParams, Patch, PatchParams, PostParams};
use kube::{Api, ResourceExt};
use proteus_core::{
    backup::gc_unreferenced, credentials_from_secret_data, LocalBackend, ObjectStore, S3Backend,
    S3Config,
};
use proteus_crd::{
    LocalBackendSpec, ProteusBackup, ProteusBackupSpec, ProteusRepository, ProteusRepositorySpec,
    ProteusRestore, ProteusRestoreSpec, RepositoryBackend, RetentionPolicy, S3BackendSpec,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tracing::warn;

use crate::error::{ApiError, ApiResult};
use crate::state::{object_namespace, ApiState};

const DEFAULT_REPO_NAMESPACE: &str = "proteus-system";
const MANAGED_SECRET_LABEL: &str = "proteus.io/managed-credentials";
const MANAGED_SECRET_REPO_LABEL: &str = "proteus.io/repository";

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
    pub pvc_names: Vec<String>,
    pub schedule: Option<String>,
    pub phase: Option<String>,
    pub message: Option<String>,
    pub last_snapshot_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub progress_percent: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_seconds: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub throughput_bytes_per_sec: Option<u64>,
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
    pub restored_snapshot_id: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateRepositoryRequest {
    pub name: String,
    #[serde(default)]
    pub namespace: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub encryption_enabled: Option<bool>,
    /// Existing Secret name; omit (with `encryptionEnabled: true`) to have Proteus generate a key.
    #[serde(default)]
    pub encryption_secret_ref: Option<String>,
    pub backend: CreateRepositoryBackend,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", tag = "type")]
pub enum CreateRepositoryBackend {
    #[serde(rename = "local")]
    Local {
        #[serde(default)]
        path: Option<String>,
    },
    #[serde(rename = "s3")]
    S3 {
        #[serde(default)]
        bucket: Option<String>,
        #[serde(default)]
        prefix: Option<String>,
        #[serde(default)]
        endpoint: Option<String>,
        #[serde(default)]
        region: Option<String>,
        /// Existing Secret name, or desired name when creating from inline keys.
        #[serde(default, rename = "credentialsSecretRef")]
        credentials_secret_ref: Option<String>,
        /// When set with `secretAccessKey`, Proteus creates/updates the Secret.
        #[serde(default, rename = "accessKeyId")]
        access_key_id: Option<String>,
        #[serde(default, rename = "secretAccessKey")]
        secret_access_key: Option<String>,
        #[serde(default)]
        force_path_style: Option<bool>,
    },
}

/// Inline S3 credentials to materialize as a Kubernetes Secret before/after CR create.
#[derive(Clone, Debug)]
pub struct InlineS3Credentials {
    pub secret_name: String,
    pub access_key_id: String,
    pub secret_access_key: String,
}

/// Generated encryption key to materialize as a Kubernetes Secret before/after CR create.
#[derive(Clone, Debug)]
pub struct InlineEncryptionKey {
    pub secret_name: String,
    pub key_base64: String,
}

#[derive(Clone, Debug)]
pub struct PreparedRepository {
    pub namespace: String,
    pub repo: ProteusRepository,
    pub inline_s3_credentials: Option<InlineS3Credentials>,
    pub inline_encryption_key: Option<InlineEncryptionKey>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PatchRepositoryRequest {
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub encryption_enabled: Option<bool>,
    #[serde(default)]
    pub backend: Option<CreateRepositoryBackend>,
}

fn repository_list_item(obj: &ProteusRepository) -> RepositoryListItem {
    let backend = match &obj.spec.backend {
        RepositoryBackend::Local(_) => Some("local".to_string()),
        RepositoryBackend::S3(_) => Some("s3".to_string()),
    };
    RepositoryListItem {
        name: obj.name_any(),
        namespace: object_namespace(obj),
        phase: obj
            .status
            .as_ref()
            .and_then(|s| s.phase.as_ref().and_then(phase_label)),
        backend,
        message: obj.status.as_ref().and_then(|s| s.message.clone()),
    }
}

fn require_non_empty(field: &str, value: Option<&str>) -> ApiResult<String> {
    match value.map(str::trim).filter(|s| !s.is_empty()) {
        Some(v) => Ok(v.to_string()),
        None => Err(ApiError::BadRequest(format!("{field} is required"))),
    }
}

fn resolve_namespace(requested: Option<&str>) -> ApiResult<String> {
    match requested.map(str::trim).filter(|s| !s.is_empty()) {
        Some(ns) => Ok(ns.to_string()),
        None => Ok(DEFAULT_REPO_NAMESPACE.to_string()),
    }
}

fn optional_trimmed(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// Resolve S3 credentials: either an existing Secret ref, or inline keys to write.
fn resolve_s3_credentials(
    repo_name: &str,
    credentials_secret_ref: Option<&str>,
    access_key_id: Option<&str>,
    secret_access_key: Option<&str>,
) -> ApiResult<(String, Option<InlineS3Credentials>)> {
    let access_key_id = optional_trimmed(access_key_id);
    let secret_access_key = optional_trimmed(secret_access_key);
    let credentials_secret_ref = optional_trimmed(credentials_secret_ref);

    match (access_key_id, secret_access_key, credentials_secret_ref) {
        (Some(access_key_id), Some(secret_access_key), secret_ref) => {
            let secret_name =
                secret_ref.unwrap_or_else(|| format!("{repo_name}-s3-creds"));
            Ok((
                secret_name.clone(),
                Some(InlineS3Credentials {
                    secret_name,
                    access_key_id,
                    secret_access_key,
                }),
            ))
        }
        (None, None, Some(secret_name)) => Ok((secret_name, None)),
        (Some(_), None, _) | (None, Some(_), _) => Err(ApiError::BadRequest(
            "backend.accessKeyId and backend.secretAccessKey must both be provided".to_string(),
        )),
        (None, None, None) => Err(ApiError::BadRequest(
            "provide backend.accessKeyId + backend.secretAccessKey, or backend.credentialsSecretRef"
                .to_string(),
        )),
    }
}

pub fn backend_from_request(
    backend: &CreateRepositoryBackend,
    repo_name: &str,
) -> ApiResult<(RepositoryBackend, Option<InlineS3Credentials>)> {
    match backend {
        CreateRepositoryBackend::Local { path } => {
            let path = require_non_empty("backend.path", path.as_deref())?;
            Ok((RepositoryBackend::Local(LocalBackendSpec { path }), None))
        }
        CreateRepositoryBackend::S3 {
            bucket,
            prefix,
            endpoint,
            region,
            credentials_secret_ref,
            access_key_id,
            secret_access_key,
            force_path_style,
        } => {
            let bucket = require_non_empty("backend.bucket", bucket.as_deref())?;
            let (credentials_secret_ref, inline) = resolve_s3_credentials(
                repo_name,
                credentials_secret_ref.as_deref(),
                access_key_id.as_deref(),
                secret_access_key.as_deref(),
            )?;
            Ok((
                RepositoryBackend::S3(S3BackendSpec {
                    bucket,
                    prefix: optional_trimmed(prefix.as_deref()),
                    endpoint: optional_trimmed(endpoint.as_deref()),
                    region: optional_trimmed(region.as_deref()),
                    credentials_secret_ref,
                    // Default true: MinIO and other S3-compatible endpoints need path-style.
                    force_path_style: force_path_style.unwrap_or(true),
                }),
                inline,
            ))
        }
    }
}

/// When `encryption_enabled`: reuse a provided Secret ref as-is, or generate a fresh key +
/// Secret name (`<repo>-encryption`) for the caller to materialize before/after CR create.
fn resolve_encryption(
    repo_name: &str,
    encryption_enabled: bool,
    encryption_secret_ref: Option<&str>,
) -> (bool, Option<String>, Option<InlineEncryptionKey>) {
    if !encryption_enabled {
        return (false, None, None);
    }
    match optional_trimmed(encryption_secret_ref) {
        Some(secret_ref) => (true, Some(secret_ref), None),
        None => {
            let secret_name = format!("{repo_name}-encryption");
            let key_base64 = proteus_core::EncryptionKey::generate()
                .to_base64()
                .to_string();
            (
                true,
                Some(secret_name.clone()),
                Some(InlineEncryptionKey {
                    secret_name,
                    key_base64,
                }),
            )
        }
    }
}

pub fn build_repository(req: &CreateRepositoryRequest) -> ApiResult<PreparedRepository> {
    let name = require_non_empty("name", Some(req.name.as_str()))?;
    let namespace = resolve_namespace(req.namespace.as_deref())?;
    let (backend, inline_s3_credentials) = backend_from_request(&req.backend, &name)?;
    let description = req
        .description
        .as_ref()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    let (encryption_enabled, encryption_secret_ref, inline_encryption_key) = resolve_encryption(
        &name,
        req.encryption_enabled.unwrap_or(false),
        req.encryption_secret_ref.as_deref(),
    );

    let repo = ProteusRepository {
        metadata: ObjectMeta {
            name: Some(name),
            namespace: Some(namespace.clone()),
            ..ObjectMeta::default()
        },
        spec: ProteusRepositorySpec {
            backend,
            description,
            encryption_enabled,
            encryption_secret_ref,
        },
        status: None,
    };
    Ok(PreparedRepository {
        namespace,
        repo,
        inline_s3_credentials,
        inline_encryption_key,
    })
}

async fn upsert_s3_credentials_secret(
    state: &ApiState,
    namespace: &str,
    creds: &InlineS3Credentials,
    owner: Option<&ProteusRepository>,
) -> ApiResult<()> {
    let mut labels = BTreeMap::new();
    labels.insert(MANAGED_SECRET_LABEL.to_string(), "true".to_string());
    if let Some(repo) = owner {
        labels.insert(MANAGED_SECRET_REPO_LABEL.to_string(), repo.name_any());
    }

    let mut string_data = BTreeMap::new();
    string_data.insert("accessKeyId".to_string(), creds.access_key_id.clone());
    string_data.insert(
        "secretAccessKey".to_string(),
        creds.secret_access_key.clone(),
    );

    let owner_references = owner.and_then(|repo| {
        let uid = repo.metadata.uid.as_ref()?;
        Some(vec![OwnerReference {
            api_version: "proteus.io/v1alpha1".to_string(),
            kind: "ProteusRepository".to_string(),
            name: repo.name_any(),
            uid: uid.clone(),
            controller: Some(true),
            block_owner_deletion: Some(true),
        }])
    });

    let secret = Secret {
        metadata: ObjectMeta {
            name: Some(creds.secret_name.clone()),
            namespace: Some(namespace.to_string()),
            labels: Some(labels),
            owner_references,
            ..ObjectMeta::default()
        },
        type_: Some("Opaque".to_string()),
        string_data: Some(string_data),
        ..Secret::default()
    };

    let api: Api<Secret> = Api::namespaced(state.client.clone(), namespace);
    match api.create(&PostParams::default(), &secret).await {
        Ok(_) => Ok(()),
        Err(kube::Error::Api(err)) if err.code == 409 => {
            // Replace credentials on conflict (UI re-submit / recreate).
            let patch = serde_json::json!({
                "metadata": {
                    "labels": {
                        MANAGED_SECRET_LABEL: "true",
                        MANAGED_SECRET_REPO_LABEL: owner.map(|r| r.name_any()).unwrap_or_default(),
                    },
                    "ownerReferences": secret.metadata.owner_references,
                },
                "type": "Opaque",
                "stringData": {
                    "accessKeyId": creds.access_key_id,
                    "secretAccessKey": creds.secret_access_key,
                }
            });
            api.patch(
                &creds.secret_name,
                &PatchParams::apply("proteus-api").force(),
                &Patch::Apply(&patch),
            )
            .await?;
            Ok(())
        }
        Err(err) => Err(err.into()),
    }
}

async fn upsert_encryption_secret(
    state: &ApiState,
    namespace: &str,
    key: &InlineEncryptionKey,
    owner: Option<&ProteusRepository>,
) -> ApiResult<()> {
    let mut labels = BTreeMap::new();
    labels.insert(MANAGED_SECRET_LABEL.to_string(), "true".to_string());
    if let Some(repo) = owner {
        labels.insert(MANAGED_SECRET_REPO_LABEL.to_string(), repo.name_any());
    }

    let mut string_data = BTreeMap::new();
    string_data.insert("encryptionKey".to_string(), key.key_base64.clone());

    let owner_references = owner.and_then(|repo| {
        let uid = repo.metadata.uid.as_ref()?;
        Some(vec![OwnerReference {
            api_version: "proteus.io/v1alpha1".to_string(),
            kind: "ProteusRepository".to_string(),
            name: repo.name_any(),
            uid: uid.clone(),
            controller: Some(true),
            block_owner_deletion: Some(true),
        }])
    });

    let secret = Secret {
        metadata: ObjectMeta {
            name: Some(key.secret_name.clone()),
            namespace: Some(namespace.to_string()),
            labels: Some(labels),
            owner_references,
            ..ObjectMeta::default()
        },
        type_: Some("Opaque".to_string()),
        string_data: Some(string_data),
        ..Secret::default()
    };

    let api: Api<Secret> = Api::namespaced(state.client.clone(), namespace);
    match api.create(&PostParams::default(), &secret).await {
        Ok(_) => Ok(()),
        Err(kube::Error::Api(err)) if err.code == 409 => {
            let patch = serde_json::json!({
                "metadata": {
                    "labels": {
                        MANAGED_SECRET_LABEL: "true",
                        MANAGED_SECRET_REPO_LABEL: owner.map(|r| r.name_any()).unwrap_or_default(),
                    },
                    "ownerReferences": secret.metadata.owner_references,
                },
                "type": "Opaque",
                "stringData": {
                    "encryptionKey": key.key_base64,
                }
            });
            api.patch(
                &key.secret_name,
                &PatchParams::apply("proteus-api").force(),
                &Patch::Apply(&patch),
            )
            .await?;
            Ok(())
        }
        Err(err) => Err(err.into()),
    }
}

/// Decode a Secret's `data`/`stringData` into raw bytes (no lossy UTF-8 coercion), so both a
/// base64 string and raw binary key material survive the round trip.
fn decode_secret_raw_data(
    data: Option<&BTreeMap<String, ByteString>>,
    string_data: Option<&BTreeMap<String, String>>,
) -> HashMap<String, Vec<u8>> {
    let mut out = HashMap::new();
    if let Some(string_data) = string_data {
        for (k, v) in string_data {
            out.insert(k.clone(), v.clone().into_bytes());
        }
    }
    if let Some(data) = data {
        for (k, v) in data {
            out.entry(k.clone()).or_insert_with(|| v.0.clone());
        }
    }
    out
}

/// Fail fast (before creating the CR) when the caller points at an existing encryption Secret
/// that is missing or does not contain a parseable key.
async fn validate_existing_encryption_secret(
    state: &ApiState,
    namespace: &str,
    secret_name: &str,
) -> ApiResult<()> {
    let api: Api<Secret> = Api::namespaced(state.client.clone(), namespace);
    let secret = api.get(secret_name).await.map_err(|_| {
        ApiError::BadRequest(format!(
            "encryptionSecretRef '{secret_name}' not found in namespace '{namespace}'"
        ))
    })?;
    let raw = decode_secret_raw_data(secret.data.as_ref(), secret.string_data.as_ref());
    proteus_core::encryption_key_from_secret_data(&raw).map_err(|err| {
        ApiError::BadRequest(format!(
            "encryptionSecretRef '{secret_name}' is invalid: {err}"
        ))
    })?;
    Ok(())
}

pub async fn list_repositories(state: &ApiState) -> ApiResult<Vec<RepositoryListItem>> {
    let api: Api<ProteusRepository> = Api::all(state.client.clone());
    let list = api.list(&ListParams::default()).await?;
    Ok(list.items.iter().map(repository_list_item).collect())
}

pub async fn get_repository(
    state: &ApiState,
    namespace: &str,
    name: &str,
) -> ApiResult<RepositoryListItem> {
    let api: Api<ProteusRepository> = Api::namespaced(state.client.clone(), namespace);
    let obj = api.get(name).await?;
    Ok(repository_list_item(&obj))
}

pub async fn create_repository(
    state: &ApiState,
    req: CreateRepositoryRequest,
) -> ApiResult<RepositoryListItem> {
    let prepared = build_repository(&req)?;
    let namespace = prepared.namespace.clone();

    // Materialize credentials Secret before the CR so the first reconcile can probe.
    if let Some(creds) = &prepared.inline_s3_credentials {
        upsert_s3_credentials_secret(state, &namespace, creds, None).await?;
    }

    match &prepared.inline_encryption_key {
        Some(key) => upsert_encryption_secret(state, &namespace, key, None).await?,
        None => {
            // A caller-supplied ref (not generated here) must already be valid.
            if let Some(secret_ref) = &prepared.repo.spec.encryption_secret_ref {
                validate_existing_encryption_secret(state, &namespace, secret_ref).await?;
            }
        }
    }

    let api: Api<ProteusRepository> = Api::namespaced(state.client.clone(), &namespace);
    let created = match api.create(&PostParams::default(), &prepared.repo).await {
        Ok(obj) => obj,
        Err(err) => {
            // Best-effort cleanup of Secrets we just created for this failed request.
            let secrets: Api<Secret> = Api::namespaced(state.client.clone(), &namespace);
            if let Some(creds) = &prepared.inline_s3_credentials {
                let _ = secrets
                    .delete(&creds.secret_name, &DeleteParams::default())
                    .await;
            }
            if let Some(key) = &prepared.inline_encryption_key {
                let _ = secrets
                    .delete(&key.secret_name, &DeleteParams::default())
                    .await;
            }
            return Err(err.into());
        }
    };

    // Attach ownerReference so the Secret is GC'd with the repository.
    if let Some(creds) = &prepared.inline_s3_credentials {
        let _ = upsert_s3_credentials_secret(state, &namespace, creds, Some(&created)).await;
    }
    if let Some(key) = &prepared.inline_encryption_key {
        let _ = upsert_encryption_secret(state, &namespace, key, Some(&created)).await;
    }

    let _ = state.refresh_counts().await;
    Ok(repository_list_item(&created))
}

pub async fn patch_repository(
    state: &ApiState,
    namespace: &str,
    name: &str,
    req: PatchRepositoryRequest,
) -> ApiResult<RepositoryListItem> {
    if req.description.is_none() && req.encryption_enabled.is_none() && req.backend.is_none() {
        return Err(ApiError::BadRequest(
            "at least one of description, encryptionEnabled, or backend is required".to_string(),
        ));
    }

    let mut patch = serde_json::Map::new();
    let mut spec = serde_json::Map::new();

    if let Some(description) = &req.description {
        let trimmed = description.trim();
        if trimmed.is_empty() {
            spec.insert("description".to_string(), Value::Null);
        } else {
            spec.insert(
                "description".to_string(),
                Value::String(trimmed.to_string()),
            );
        }
    }
    if let Some(enabled) = req.encryption_enabled {
        spec.insert("encryptionEnabled".to_string(), Value::Bool(enabled));
    }
    let mut inline_creds = None;
    if let Some(backend) = &req.backend {
        let (backend, inline) = backend_from_request(backend, name)?;
        inline_creds = inline;
        spec.insert(
            "backend".to_string(),
            serde_json::to_value(backend)
                .map_err(|err| ApiError::Internal(format!("failed to serialize backend: {err}")))?,
        );
    }

    patch.insert("spec".to_string(), Value::Object(spec));
    let body = Value::Object(patch);

    let api: Api<ProteusRepository> = Api::namespaced(state.client.clone(), namespace);
    let updated = api
        .patch(name, &PatchParams::default(), &Patch::Merge(&body))
        .await?;

    if let Some(creds) = inline_creds {
        upsert_s3_credentials_secret(state, namespace, &creds, Some(&updated)).await?;
    }

    Ok(repository_list_item(&updated))
}

pub async fn delete_repository(state: &ApiState, namespace: &str, name: &str) -> ApiResult<()> {
    let api: Api<ProteusRepository> = Api::namespaced(state.client.clone(), namespace);
    api.delete(name, &DeleteParams::default()).await?;
    let _ = state.refresh_counts().await;
    Ok(())
}

fn backup_list_item(obj: &ProteusBackup) -> BackupListItem {
    BackupListItem {
        name: obj.name_any(),
        namespace: object_namespace(obj),
        repository_ref: obj.spec.repository_ref.clone(),
        target_namespace: obj.spec.target_namespace.clone(),
        pvc_names: obj.spec.pvc_names.clone(),
        schedule: obj.spec.schedule.clone(),
        phase: obj
            .status
            .as_ref()
            .and_then(|s| s.phase.as_ref().and_then(phase_label)),
        message: obj.status.as_ref().and_then(|s| s.message.clone()),
        last_snapshot_id: obj.status.as_ref().and_then(|s| s.last_snapshot_id.clone()),
        progress_percent: obj.status.as_ref().and_then(|s| s.progress_percent),
        duration_seconds: obj.status.as_ref().and_then(|s| s.duration_seconds),
        throughput_bytes_per_sec: obj.status.as_ref().and_then(|s| s.throughput_bytes_per_sec),
    }
}

pub async fn list_backups(state: &ApiState) -> ApiResult<Vec<BackupListItem>> {
    let api: Api<ProteusBackup> = Api::all(state.client.clone());
    let list = api.list(&ListParams::default()).await?;
    Ok(list.items.iter().map(backup_list_item).collect())
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateBackupRequest {
    pub name: String,
    /// Namespace for the `ProteusBackup` CR itself; defaults to `targetNamespace`.
    #[serde(default)]
    pub namespace: Option<String>,
    pub repository_ref: String,
    #[serde(default)]
    pub repository_namespace: Option<String>,
    pub target_namespace: String,
    #[serde(default)]
    pub pvc_names: Vec<String>,
}

pub fn build_backup(req: &CreateBackupRequest) -> ApiResult<(String, ProteusBackup)> {
    let name = require_non_empty("name", Some(req.name.as_str()))?;
    let target_namespace =
        require_non_empty("targetNamespace", Some(req.target_namespace.as_str()))?;
    let repository_ref = require_non_empty("repositoryRef", Some(req.repository_ref.as_str()))?;
    let namespace =
        optional_trimmed(req.namespace.as_deref()).unwrap_or_else(|| target_namespace.clone());

    let pvc_names: Vec<String> = req
        .pvc_names
        .iter()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    if pvc_names.is_empty() {
        return Err(ApiError::BadRequest(
            "pvcNames must contain at least one PVC name".to_string(),
        ));
    }

    let backup = ProteusBackup {
        metadata: ObjectMeta {
            name: Some(name),
            namespace: Some(namespace.clone()),
            ..ObjectMeta::default()
        },
        spec: ProteusBackupSpec {
            repository_ref,
            repository_namespace: optional_trimmed(req.repository_namespace.as_deref()),
            target_namespace,
            pvc_names,
            label_selector: None,
            schedule: None,
            retention: RetentionPolicy::default(),
            include_volumes: true,
            include_cluster_resources: false,
        },
        status: None,
    };
    Ok((namespace, backup))
}

pub async fn create_backup(
    state: &ApiState,
    req: CreateBackupRequest,
) -> ApiResult<BackupListItem> {
    let (namespace, backup) = build_backup(&req)?;
    let api: Api<ProteusBackup> = Api::namespaced(state.client.clone(), &namespace);
    let created = api.create(&PostParams::default(), &backup).await?;
    let _ = state.refresh_counts().await;
    Ok(backup_list_item(&created))
}

pub async fn delete_backup(state: &ApiState, namespace: &str, name: &str) -> ApiResult<()> {
    let api: Api<ProteusBackup> = Api::namespaced(state.client.clone(), namespace);
    let backup = api.get(name).await?;

    let repo_ns = backup
        .spec
        .repository_namespace
        .as_deref()
        .unwrap_or(namespace)
        .to_string();
    let repo_ref = backup.spec.repository_ref.clone();

    // GC first: drop this backup's snapshot + orphans; keep other backups' objects.
    match gc_repository_after_backup_delete(state, namespace, name, &repo_ns, &repo_ref).await {
        Ok(removed) => {
            if removed > 0 {
                tracing::info!(
                    backup = %name,
                    repository = %repo_ref,
                    removed,
                    "purged unreferenced objects from repository"
                );
            }
        }
        Err(err) => {
            warn!(
                backup = %name,
                error = %err,
                "failed to purge repository objects; leaving CR in place"
            );
            return Err(ApiError::Internal(format!(
                "could not remove backup data from repository: {err}"
            )));
        }
    }

    api.delete(name, &DeleteParams::default()).await?;
    let _ = state.refresh_counts().await;
    Ok(())
}

/// Keep snapshots belonging to other backups that share the same repository; delete the rest.
async fn gc_repository_after_backup_delete(
    state: &ApiState,
    deleting_namespace: &str,
    deleting_name: &str,
    repo_namespace: &str,
    repo_ref: &str,
) -> Result<u64, String> {
    let all: Api<ProteusBackup> = Api::all(state.client.clone());
    let list = all
        .list(&ListParams::default())
        .await
        .map_err(|err| format!("failed to list backups for GC: {err}"))?;

    let mut keep_snapshots = Vec::new();
    for other in &list.items {
        let other_ns = object_namespace(other);
        let other_name = other.name_any();
        if other_ns == deleting_namespace && other_name == deleting_name {
            continue;
        }
        let other_repo_ns = other
            .spec
            .repository_namespace
            .as_deref()
            .unwrap_or(other_ns.as_str());
        if other.spec.repository_ref != repo_ref || other_repo_ns != repo_namespace {
            continue;
        }
        if let Some(id) = other
            .status
            .as_ref()
            .and_then(|s| s.last_snapshot_id.as_ref())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
        {
            keep_snapshots.push(id);
        }
    }

    let store = open_repository_store(state, repo_namespace, repo_ref).await?;
    gc_unreferenced(store.as_ref(), &keep_snapshots)
        .await
        .map_err(|err| err.to_string())
}

async fn open_repository_store(
    state: &ApiState,
    namespace: &str,
    repo_ref: &str,
) -> Result<Arc<dyn ObjectStore>, String> {
    let api: Api<ProteusRepository> = Api::namespaced(state.client.clone(), namespace);
    let repo = api
        .get(repo_ref)
        .await
        .map_err(|err| format!("repository '{repo_ref}' not found in '{namespace}': {err}"))?;

    match &repo.spec.backend {
        RepositoryBackend::Local(local) => {
            let backend = LocalBackend::open(&local.path)
                .await
                .map_err(|err| format!("failed to open local repository: {err}"))?;
            Ok(Arc::new(backend))
        }
        RepositoryBackend::S3(s3) => {
            let credentials =
                load_s3_credentials_for_api(state, namespace, &s3.credentials_secret_ref).await?;
            let backend = S3Backend::new(
                S3Config {
                    bucket: s3.bucket.clone(),
                    prefix: s3.prefix.clone(),
                    endpoint: s3.endpoint.clone(),
                    region: s3.region.clone(),
                    force_path_style: s3.force_path_style,
                },
                credentials,
            )
            .map_err(|err| format!("failed to build S3 client: {err}"))?;
            Ok(Arc::new(backend))
        }
    }
}

async fn load_s3_credentials_for_api(
    state: &ApiState,
    namespace: &str,
    secret_name: &str,
) -> Result<proteus_core::S3Credentials, String> {
    let secrets: Api<Secret> = Api::namespaced(state.client.clone(), namespace);
    let secret = secrets
        .get(secret_name)
        .await
        .map_err(|err| format!("credentials Secret '{secret_name}' not found: {err}"))?;
    let decoded = decode_secret_data(secret.data.as_ref(), secret.string_data.as_ref());
    credentials_from_secret_data(&decoded).map_err(|err| err.to_string())
}

fn decode_secret_data(
    data: Option<&BTreeMap<String, ByteString>>,
    string_data: Option<&BTreeMap<String, String>>,
) -> HashMap<String, String> {
    let mut out = HashMap::new();
    if let Some(string_data) = string_data {
        for (k, v) in string_data {
            out.insert(k.clone(), v.clone());
        }
    }
    if let Some(data) = data {
        for (k, v) in data {
            if let Ok(s) = String::from_utf8(v.0.clone()) {
                out.entry(k.clone()).or_insert(s);
            }
        }
    }
    out
}

fn restore_list_item(obj: &ProteusRestore) -> RestoreListItem {
    RestoreListItem {
        name: obj.name_any(),
        namespace: object_namespace(obj),
        backup_ref: obj.spec.backup_ref.clone(),
        target_namespace: obj.spec.target_namespace.clone(),
        phase: obj
            .status
            .as_ref()
            .and_then(|s| s.phase.as_ref().and_then(phase_label)),
        message: obj.status.as_ref().and_then(|s| s.message.clone()),
        restored_snapshot_id: obj
            .status
            .as_ref()
            .and_then(|s| s.restored_snapshot_id.clone()),
    }
}

pub async fn list_restores(state: &ApiState) -> ApiResult<Vec<RestoreListItem>> {
    let api: Api<ProteusRestore> = Api::all(state.client.clone());
    let list = api.list(&ListParams::default()).await?;
    Ok(list.items.iter().map(restore_list_item).collect())
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateRestoreRequest {
    pub name: String,
    /// Namespace for the `ProteusRestore` CR itself; defaults to `targetNamespace`.
    #[serde(default)]
    pub namespace: Option<String>,
    pub backup_ref: String,
    /// Namespace of the source `ProteusBackup`; defaults to the restore's own namespace.
    #[serde(default)]
    pub backup_namespace: Option<String>,
    /// Explicit snapshot id; omit to let the controller resolve the backup's latest snapshot.
    #[serde(default)]
    pub snapshot_id: Option<String>,
    pub target_namespace: String,
    #[serde(default)]
    pub overwrite: bool,
}

pub fn build_restore(req: &CreateRestoreRequest) -> ApiResult<(String, ProteusRestore)> {
    let name = require_non_empty("name", Some(req.name.as_str()))?;
    let target_namespace =
        require_non_empty("targetNamespace", Some(req.target_namespace.as_str()))?;
    let backup_ref = require_non_empty("backupRef", Some(req.backup_ref.as_str()))?;
    let namespace =
        optional_trimmed(req.namespace.as_deref()).unwrap_or_else(|| target_namespace.clone());

    let restore = ProteusRestore {
        metadata: ObjectMeta {
            name: Some(name),
            namespace: Some(namespace.clone()),
            ..ObjectMeta::default()
        },
        spec: ProteusRestoreSpec {
            backup_ref,
            backup_namespace: optional_trimmed(req.backup_namespace.as_deref()),
            snapshot_id: optional_trimmed(req.snapshot_id.as_deref()),
            target_namespace,
            overwrite: req.overwrite,
            include_resources: None,
        },
        status: None,
    };
    Ok((namespace, restore))
}

pub async fn create_restore(
    state: &ApiState,
    req: CreateRestoreRequest,
) -> ApiResult<RestoreListItem> {
    let (namespace, restore) = build_restore(&req)?;
    let api: Api<ProteusRestore> = Api::namespaced(state.client.clone(), &namespace);
    let created = api.create(&PostParams::default(), &restore).await?;
    let _ = state.refresh_counts().await;
    Ok(restore_list_item(&created))
}

pub async fn delete_restore(state: &ApiState, namespace: &str, name: &str) -> ApiResult<()> {
    let api: Api<ProteusRestore> = Api::namespaced(state.client.clone(), namespace);
    api.delete(name, &DeleteParams::default()).await?;
    let _ = state.refresh_counts().await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s3_backend(
        bucket: Option<&str>,
        credentials_secret_ref: Option<&str>,
        access_key_id: Option<&str>,
        secret_access_key: Option<&str>,
        force_path_style: Option<bool>,
    ) -> CreateRepositoryBackend {
        CreateRepositoryBackend::S3 {
            bucket: bucket.map(str::to_string),
            prefix: None,
            endpoint: None,
            region: None,
            credentials_secret_ref: credentials_secret_ref.map(str::to_string),
            access_key_id: access_key_id.map(str::to_string),
            secret_access_key: secret_access_key.map(str::to_string),
            force_path_style,
        }
    }

    #[test]
    fn rejects_missing_local_path() {
        let req = CreateRepositoryRequest {
            name: "r1".into(),
            namespace: None,
            description: None,
            encryption_enabled: None,
            encryption_secret_ref: None,
            backend: CreateRepositoryBackend::Local { path: None },
        };
        let err = build_repository(&req).expect_err("path required");
        assert!(err.to_string().contains("backend.path"));
    }

    #[test]
    fn rejects_empty_s3_bucket() {
        let req = CreateRepositoryRequest {
            name: "r1".into(),
            namespace: Some("proteus-system".into()),
            description: None,
            encryption_enabled: None,
            encryption_secret_ref: None,
            backend: s3_backend(Some("  "), Some("s3-creds"), None, None, None),
        };
        let err = build_repository(&req).expect_err("bucket required");
        assert!(err.to_string().contains("backend.bucket"));
    }

    #[test]
    fn rejects_missing_s3_credentials() {
        let req = CreateRepositoryRequest {
            name: "r1".into(),
            namespace: None,
            description: None,
            encryption_enabled: None,
            encryption_secret_ref: None,
            backend: s3_backend(Some("backups"), None, None, None, Some(true)),
        };
        let err = build_repository(&req).expect_err("credentials required");
        assert!(err.to_string().contains("accessKeyId"));
    }

    #[test]
    fn rejects_partial_inline_s3_credentials() {
        let req = CreateRepositoryRequest {
            name: "r1".into(),
            namespace: None,
            description: None,
            encryption_enabled: None,
            encryption_secret_ref: None,
            backend: s3_backend(Some("backups"), None, Some("AKIA"), None, None),
        };
        let err = build_repository(&req).expect_err("both keys required");
        assert!(err.to_string().contains("must both be provided"));
    }

    #[test]
    fn builds_local_repo_with_default_namespace() {
        let req = CreateRepositoryRequest {
            name: "local-1".into(),
            namespace: None,
            description: Some("dev".into()),
            encryption_enabled: Some(true),
            encryption_secret_ref: None,
            backend: CreateRepositoryBackend::Local {
                path: Some("/var/lib/proteus/repo".into()),
            },
        };
        let prepared = build_repository(&req).expect("valid");
        assert_eq!(prepared.namespace, "proteus-system");
        assert_eq!(prepared.repo.metadata.name.as_deref(), Some("local-1"));
        assert_eq!(
            prepared.repo.metadata.namespace.as_deref(),
            Some("proteus-system")
        );
        assert!(prepared.repo.spec.encryption_enabled);
        assert!(prepared.inline_s3_credentials.is_none());
        assert!(matches!(
            prepared.repo.spec.backend,
            RepositoryBackend::Local(ref local) if local.path == "/var/lib/proteus/repo"
        ));
    }

    #[test]
    fn builds_s3_repo_from_existing_secret_ref() {
        let req = CreateRepositoryRequest {
            name: "s3-1".into(),
            namespace: Some("demo".into()),
            description: None,
            encryption_enabled: None,
            encryption_secret_ref: None,
            backend: CreateRepositoryBackend::S3 {
                bucket: Some("proteus".into()),
                prefix: Some("cas/".into()),
                endpoint: Some("http://minio.local:9000".into()),
                region: Some("us-east-1".into()),
                credentials_secret_ref: Some("minio-creds".into()),
                access_key_id: None,
                secret_access_key: None,
                force_path_style: Some(true),
            },
        };
        let prepared = build_repository(&req).expect("valid");
        assert_eq!(prepared.namespace, "demo");
        assert!(prepared.inline_s3_credentials.is_none());
        assert!(matches!(
            prepared.repo.spec.backend,
            RepositoryBackend::S3(ref s3)
                if s3.bucket == "proteus"
                    && s3.credentials_secret_ref == "minio-creds"
                    && s3.force_path_style
        ));
    }

    #[test]
    fn builds_s3_repo_from_inline_keys_with_default_secret_name() {
        let req = CreateRepositoryRequest {
            name: "scw-repo".into(),
            namespace: Some("default".into()),
            description: None,
            encryption_enabled: None,
            encryption_secret_ref: None,
            backend: s3_backend(
                Some("my-bucket"),
                None,
                Some("SCWXXXX"),
                Some("secret-value"),
                Some(true),
            ),
        };
        let prepared = build_repository(&req).expect("valid");
        let inline = prepared.inline_s3_credentials.expect("inline creds");
        assert_eq!(inline.secret_name, "scw-repo-s3-creds");
        assert_eq!(inline.access_key_id, "SCWXXXX");
        assert!(matches!(
            prepared.repo.spec.backend,
            RepositoryBackend::S3(ref s3) if s3.credentials_secret_ref == "scw-repo-s3-creds"
        ));
    }

    #[test]
    fn s3_force_path_style_defaults_true_when_omitted() {
        let req = CreateRepositoryRequest {
            name: "s3-default".into(),
            namespace: None,
            description: None,
            encryption_enabled: None,
            encryption_secret_ref: None,
            backend: s3_backend(Some("proteus"), Some("minio-creds"), None, None, None),
        };
        let prepared = build_repository(&req).expect("valid");
        assert!(matches!(
            prepared.repo.spec.backend,
            RepositoryBackend::S3(ref s3) if s3.force_path_style
        ));
    }

    #[test]
    fn resolve_encryption_generates_key_when_enabled_without_ref() {
        let (enabled, secret_ref, inline) = resolve_encryption("repo-1", true, None);
        assert!(enabled);
        assert_eq!(secret_ref.as_deref(), Some("repo-1-encryption"));
        let inline = inline.expect("generated key");
        assert_eq!(inline.secret_name, "repo-1-encryption");
        assert!(!inline.key_base64.is_empty());
    }

    #[test]
    fn resolve_encryption_reuses_provided_ref_without_generating() {
        let (enabled, secret_ref, inline) =
            resolve_encryption("repo-1", true, Some("existing-key"));
        assert!(enabled);
        assert_eq!(secret_ref.as_deref(), Some("existing-key"));
        assert!(inline.is_none());
    }

    #[test]
    fn resolve_encryption_disabled_ignores_ref() {
        let (enabled, secret_ref, inline) = resolve_encryption("repo-1", false, Some("ignored"));
        assert!(!enabled);
        assert!(secret_ref.is_none());
        assert!(inline.is_none());
    }

    #[test]
    fn build_backup_rejects_empty_pvc_names() {
        let req = CreateBackupRequest {
            name: "b1".into(),
            namespace: None,
            repository_ref: "repo-1".into(),
            repository_namespace: None,
            target_namespace: "workloads".into(),
            pvc_names: vec![],
        };
        let err = build_backup(&req).expect_err("pvcNames required");
        assert!(err.to_string().contains("pvcNames"));
    }

    #[test]
    fn build_backup_rejects_blank_pvc_names() {
        let req = CreateBackupRequest {
            name: "b1".into(),
            namespace: None,
            repository_ref: "repo-1".into(),
            repository_namespace: None,
            target_namespace: "workloads".into(),
            pvc_names: vec!["  ".into()],
        };
        let err = build_backup(&req).expect_err("blank pvc name filtered out");
        assert!(err.to_string().contains("pvcNames"));
    }

    #[test]
    fn build_backup_defaults_namespace_to_target_namespace() {
        let req = CreateBackupRequest {
            name: "b1".into(),
            namespace: None,
            repository_ref: "repo-1".into(),
            repository_namespace: None,
            target_namespace: "workloads".into(),
            pvc_names: vec!["data-pvc".into()],
        };
        let (namespace, backup) = build_backup(&req).expect("valid");
        assert_eq!(namespace, "workloads");
        assert_eq!(backup.metadata.namespace.as_deref(), Some("workloads"));
        assert_eq!(backup.spec.pvc_names, vec!["data-pvc".to_string()]);
    }

    #[test]
    fn build_backup_honours_explicit_namespace() {
        let req = CreateBackupRequest {
            name: "b1".into(),
            namespace: Some("proteus-system".into()),
            repository_ref: "repo-1".into(),
            repository_namespace: Some("proteus-system".into()),
            target_namespace: "workloads".into(),
            pvc_names: vec!["data-pvc".into(), "logs-pvc".into()],
        };
        let (namespace, backup) = build_backup(&req).expect("valid");
        assert_eq!(namespace, "proteus-system");
        assert_eq!(backup.spec.target_namespace, "workloads");
        assert_eq!(backup.spec.pvc_names.len(), 2);
    }

    #[test]
    fn build_restore_rejects_empty_backup_ref() {
        let req = CreateRestoreRequest {
            name: "r1".into(),
            namespace: None,
            backup_ref: "  ".into(),
            backup_namespace: None,
            snapshot_id: None,
            target_namespace: "workloads".into(),
            overwrite: false,
        };
        let err = build_restore(&req).expect_err("backupRef required");
        assert!(err.to_string().contains("backupRef"));
    }

    #[test]
    fn build_restore_rejects_empty_target_namespace() {
        let req = CreateRestoreRequest {
            name: "r1".into(),
            namespace: None,
            backup_ref: "backup-1".into(),
            backup_namespace: None,
            snapshot_id: None,
            target_namespace: "".into(),
            overwrite: false,
        };
        let err = build_restore(&req).expect_err("targetNamespace required");
        assert!(err.to_string().contains("targetNamespace"));
    }

    #[test]
    fn build_restore_defaults_namespace_to_target_namespace() {
        let req = CreateRestoreRequest {
            name: "r1".into(),
            namespace: None,
            backup_ref: "backup-1".into(),
            backup_namespace: None,
            snapshot_id: None,
            target_namespace: "workloads".into(),
            overwrite: true,
        };
        let (namespace, restore) = build_restore(&req).expect("valid");
        assert_eq!(namespace, "workloads");
        assert_eq!(restore.metadata.namespace.as_deref(), Some("workloads"));
        assert!(restore.spec.overwrite);
        assert!(restore.spec.snapshot_id.is_none());
    }

    #[test]
    fn build_restore_honours_cross_namespace_backup_ref() {
        let req = CreateRestoreRequest {
            name: "r1".into(),
            namespace: Some("proteus-system".into()),
            backup_ref: "backup-1".into(),
            backup_namespace: Some("proteus-system".into()),
            snapshot_id: Some("deadbeef".into()),
            target_namespace: "workloads".into(),
            overwrite: false,
        };
        let (namespace, restore) = build_restore(&req).expect("valid");
        assert_eq!(namespace, "proteus-system");
        assert_eq!(
            restore.spec.backup_namespace.as_deref(),
            Some("proteus-system")
        );
        assert_eq!(restore.spec.snapshot_id.as_deref(), Some("deadbeef"));
        assert_eq!(restore.spec.target_namespace, "workloads");
    }

    #[test]
    fn s3_force_path_style_false_when_explicit() {
        let req = CreateRepositoryRequest {
            name: "s3-aws".into(),
            namespace: None,
            description: None,
            encryption_enabled: None,
            encryption_secret_ref: None,
            backend: s3_backend(Some("proteus"), Some("aws-creds"), None, None, Some(false)),
        };
        let prepared = build_repository(&req).expect("valid");
        assert!(matches!(
            prepared.repo.spec.backend,
            RepositoryBackend::S3(ref s3) if !s3.force_path_style
        ));
    }
}

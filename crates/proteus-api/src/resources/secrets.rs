use std::collections::{BTreeMap, HashMap};

use k8s_openapi::api::core::v1::Secret;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::{ObjectMeta, OwnerReference};
use k8s_openapi::ByteString;
use kube::api::{Patch, PatchParams, PostParams};
use kube::{Api, ResourceExt};
use proteus_crd::ProteusRepository;

use crate::error::{ApiError, ApiResult};
use crate::state::ApiState;

const MANAGED_SECRET_LABEL: &str = "proteus.io/managed-credentials";
const MANAGED_SECRET_REPO_LABEL: &str = "proteus.io/repository";

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

pub(crate) async fn upsert_s3_credentials_secret(
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

pub(crate) async fn upsert_encryption_secret(
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
pub(crate) async fn validate_existing_encryption_secret(
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

pub(crate) async fn load_s3_credentials_for_api(
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
    proteus_core::credentials_from_secret_data(&decoded).map_err(|err| err.to_string())
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

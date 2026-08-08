use std::collections::HashMap;
use std::sync::Arc;

use k8s_openapi::api::core::v1::Secret;
use k8s_openapi::ByteString;
use kube::{Api, Client};
use proteus_core::{
    encryption_key_from_secret_data, EncryptionKey, LocalBackend, ObjectStore, S3Backend,
};
use proteus_crd::{ProteusRepository, RepositoryBackend, RepositoryPhase};
use std::collections::BTreeMap;

use crate::controllers::repository::{load_s3_credentials, s3_config_from_spec};

/// A repository ready to receive a snapshot: its store, and encryption key if enabled.
pub struct OpenedRepository {
    pub store: Arc<dyn ObjectStore>,
    pub encryption_key: Option<EncryptionKey>,
}

/// Resolve `repo_ref` (in `repo_namespace`, falling back to `default_namespace`), require it be
/// `Ready`, build its object store, and load the encryption key if `encryptionEnabled`.
pub async fn open_repository(
    client: &Client,
    default_namespace: &str,
    repo_ref: &str,
    repo_namespace: Option<&str>,
) -> Result<OpenedRepository, String> {
    let ns = repo_namespace.unwrap_or(default_namespace);
    let api: Api<ProteusRepository> = Api::namespaced(client.clone(), ns);
    let repo = api
        .get(repo_ref)
        .await
        .map_err(|err| format!("repository '{repo_ref}' not found in namespace '{ns}': {err}"))?;

    let phase = repo.status.as_ref().and_then(|s| s.phase.as_ref());
    if !matches!(phase, Some(RepositoryPhase::Ready)) {
        return Err(format!(
            "repository '{repo_ref}' is not Ready (phase={phase:?})"
        ));
    }

    let store: Arc<dyn ObjectStore> = match &repo.spec.backend {
        RepositoryBackend::Local(local) => Arc::new(
            LocalBackend::open(&local.path)
                .await
                .map_err(|err| format!("failed to open local repository: {err}"))?,
        ),
        RepositoryBackend::S3(s3) => {
            let credentials = load_s3_credentials(client, ns, &s3.credentials_secret_ref).await?;
            let backend = S3Backend::new(s3_config_from_spec(s3), credentials)
                .map_err(|err| format!("failed to build S3 client: {err}"))?;
            Arc::new(backend)
        }
    };

    let encryption_key = if repo.spec.encryption_enabled {
        let secret_name = repo.spec.encryption_secret_ref.as_deref().ok_or_else(|| {
            format!("repository '{repo_ref}' has encryptionEnabled but no encryptionSecretRef")
        })?;
        Some(load_encryption_key(client, ns, secret_name).await?)
    } else {
        None
    };

    Ok(OpenedRepository {
        store,
        encryption_key,
    })
}

/// Classify whether a repository is agent-reachable (`Remote`) or controller-local only.
pub async fn repository_kind(
    client: &Client,
    default_namespace: &str,
    repo_ref: &str,
    repo_namespace: Option<&str>,
) -> Result<crate::data_plane::RepositoryKind, String> {
    let ns = repo_namespace.unwrap_or(default_namespace);
    let api: Api<ProteusRepository> = Api::namespaced(client.clone(), ns);
    let repo = api
        .get(repo_ref)
        .await
        .map_err(|err| format!("repository '{repo_ref}' not found in namespace '{ns}': {err}"))?;
    Ok(match repo.spec.backend {
        RepositoryBackend::Local(_) => crate::data_plane::RepositoryKind::Local,
        RepositoryBackend::S3(_) => crate::data_plane::RepositoryKind::Remote,
    })
}

async fn load_encryption_key(
    client: &Client,
    namespace: &str,
    secret_name: &str,
) -> Result<EncryptionKey, String> {
    let secrets: Api<Secret> = Api::namespaced(client.clone(), namespace);
    let secret = secrets.get(secret_name).await.map_err(|err| {
        format!("encryption Secret '{secret_name}' not found in namespace '{namespace}': {err}")
    })?;
    let raw = decode_secret_raw_data(secret.data.as_ref(), secret.string_data.as_ref());
    encryption_key_from_secret_data(&raw).map_err(|err| {
        format!(
            "encryption Secret '{secret_name}' is invalid: {err}. {}",
            repo_hint(secret_name)
        )
    })
}

fn repo_hint(secret_name: &str) -> String {
    format!("expected a 32-byte or base64-encoded 'encryptionKey' in Secret '{secret_name}'")
}

/// Decode raw bytes (no lossy UTF-8 coercion) so both base64 text and raw binary keys survive.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_prefers_string_data_over_data() {
        let mut string_data = BTreeMap::new();
        string_data.insert("encryptionKey".to_string(), "from-string".to_string());
        let decoded = decode_secret_raw_data(None, Some(&string_data));
        assert_eq!(
            decoded.get("encryptionKey").map(|v| v.as_slice()),
            Some(b"from-string".as_slice())
        );
    }
}

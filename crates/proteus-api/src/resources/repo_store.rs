use std::sync::Arc;

use kube::api::ListParams;
use kube::{Api, ResourceExt};
use proteus_core::{backup::gc_unreferenced, LocalBackend, ObjectStore, S3Backend, S3Config};
use proteus_crd::{ProteusBackup, ProteusRepository, RepositoryBackend};

use super::secrets::load_s3_credentials_for_api;
use crate::state::{object_namespace, ApiState};

/// Keep snapshots belonging to other backups that share the same repository; delete the rest.
pub(crate) async fn gc_repository_after_backup_delete(
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

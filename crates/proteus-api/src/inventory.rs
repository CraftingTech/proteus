use std::fmt::Debug;

use k8s_openapi::api::apps::v1::Deployment;
use k8s_openapi::api::core::v1::{ConfigMap, PersistentVolumeClaim, Pod, Secret, Service};
use k8s_openapi::NamespaceResourceScope;
use kube::api::{ListParams, PartialObjectMeta};
use kube::{Api, Resource, ResourceExt};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::error::{ApiError, ApiResult};
use crate::state::{object_namespace, ApiState};

pub const INVENTORY_KINDS: &[&str] = &[
    "Deployment",
    "Pod",
    "Service",
    "PersistentVolumeClaim",
    "ConfigMap",
    "Secret",
];

#[derive(Clone, Debug, Default, Deserialize)]
pub struct InventoryQuery {
    pub namespace: Option<String>,
    pub kind: Option<String>,
    pub q: Option<String>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct InventoryItem {
    pub kind: String,
    pub name: String,
    pub namespace: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extra: Option<String>,
}

pub async fn list_inventory(
    state: &ApiState,
    query: &InventoryQuery,
) -> ApiResult<Vec<InventoryItem>> {
    let kinds = resolve_kinds(query.kind.as_deref())?;
    let mut items = Vec::new();

    for kind in kinds {
        items.extend(list_kind(state, kind, query.namespace.as_deref()).await?);
    }

    Ok(filter_by_name(items, query.q.as_deref()))
}

fn resolve_kinds(kind: Option<&str>) -> ApiResult<Vec<&'static str>> {
    match kind.map(str::trim).filter(|s| !s.is_empty()) {
        None | Some("All") | Some("all") | Some("*") => Ok(INVENTORY_KINDS.to_vec()),
        Some(raw) => {
            let matched = INVENTORY_KINDS
                .iter()
                .copied()
                .find(|allowed| allowed.eq_ignore_ascii_case(raw) || pvc_alias(raw, allowed));
            match matched {
                Some(kind) => Ok(vec![kind]),
                None => Err(ApiError::BadRequest(format!(
                    "unsupported inventory kind '{raw}'; allowed: {}",
                    INVENTORY_KINDS.join(", ")
                ))),
            }
        }
    }
}

fn pvc_alias(raw: &str, allowed: &str) -> bool {
    allowed == "PersistentVolumeClaim" && raw.eq_ignore_ascii_case("PVC")
}

fn filter_by_name(items: Vec<InventoryItem>, q: Option<&str>) -> Vec<InventoryItem> {
    let Some(needle) = q.map(str::trim).filter(|s| !s.is_empty()) else {
        return items;
    };
    let needle = needle.to_ascii_lowercase();
    items
        .into_iter()
        .filter(|item| item.name.to_ascii_lowercase().contains(&needle))
        .collect()
}

async fn list_kind(
    state: &ApiState,
    kind: &str,
    namespace: Option<&str>,
) -> ApiResult<Vec<InventoryItem>> {
    match kind {
        "Deployment" => list_deployments(state, namespace).await,
        "Pod" => list_pods(state, namespace).await,
        "Service" => list_services(state, namespace).await,
        "PersistentVolumeClaim" => list_pvcs(state, namespace).await,
        "ConfigMap" => list_configmaps(state, namespace).await,
        "Secret" => list_secrets(state, namespace).await,
        other => Err(ApiError::Internal(format!(
            "unhandled inventory kind {other}"
        ))),
    }
}

async fn list_objects<K>(state: &ApiState, namespace: Option<&str>) -> ApiResult<Vec<K>>
where
    K: Resource<Scope = NamespaceResourceScope> + Clone + DeserializeOwned + Debug,
    <K as Resource>::DynamicType: Default,
{
    let list = match namespace.filter(|ns| !ns.is_empty()) {
        Some(ns) => {
            Api::<K>::namespaced(state.client.clone(), ns)
                .list(&ListParams::default())
                .await?
        }
        None => {
            Api::<K>::all(state.client.clone())
                .list(&ListParams::default())
                .await?
        }
    };
    Ok(list.items)
}

async fn list_object_metadata<K>(
    state: &ApiState,
    namespace: Option<&str>,
) -> ApiResult<Vec<PartialObjectMeta<K>>>
where
    K: Resource<Scope = NamespaceResourceScope> + Clone + DeserializeOwned + Debug,
    <K as Resource>::DynamicType: Default,
{
    let list = match namespace.filter(|ns| !ns.is_empty()) {
        Some(ns) => {
            Api::<K>::namespaced(state.client.clone(), ns)
                .list_metadata(&ListParams::default())
                .await?
        }
        None => {
            Api::<K>::all(state.client.clone())
                .list_metadata(&ListParams::default())
                .await?
        }
    };
    Ok(list.items)
}

async fn list_deployments(
    state: &ApiState,
    namespace: Option<&str>,
) -> ApiResult<Vec<InventoryItem>> {
    let items = list_objects::<Deployment>(state, namespace).await?;
    Ok(items
        .into_iter()
        .map(|obj| {
            let ready = obj
                .status
                .as_ref()
                .and_then(|s| s.ready_replicas)
                .unwrap_or(0);
            let desired = obj.spec.as_ref().and_then(|s| s.replicas).unwrap_or(0);
            InventoryItem {
                kind: "Deployment".to_string(),
                name: obj.name_any(),
                namespace: object_namespace(&obj),
                extra: Some(format!("{ready}/{desired} ready")),
            }
        })
        .collect())
}

async fn list_pods(state: &ApiState, namespace: Option<&str>) -> ApiResult<Vec<InventoryItem>> {
    let items = list_objects::<Pod>(state, namespace).await?;
    Ok(items
        .into_iter()
        .map(|obj| {
            let phase = obj
                .status
                .as_ref()
                .and_then(|s| s.phase.clone())
                .unwrap_or_else(|| "Unknown".to_string());
            InventoryItem {
                kind: "Pod".to_string(),
                name: obj.name_any(),
                namespace: object_namespace(&obj),
                extra: Some(phase),
            }
        })
        .collect())
}

async fn list_services(state: &ApiState, namespace: Option<&str>) -> ApiResult<Vec<InventoryItem>> {
    let items = list_objects::<Service>(state, namespace).await?;
    Ok(items
        .into_iter()
        .map(|obj| {
            let svc_type = obj
                .spec
                .as_ref()
                .and_then(|s| s.type_.clone())
                .unwrap_or_else(|| "ClusterIP".to_string());
            InventoryItem {
                kind: "Service".to_string(),
                name: obj.name_any(),
                namespace: object_namespace(&obj),
                extra: Some(svc_type),
            }
        })
        .collect())
}

async fn list_pvcs(state: &ApiState, namespace: Option<&str>) -> ApiResult<Vec<InventoryItem>> {
    let items = list_objects::<PersistentVolumeClaim>(state, namespace).await?;
    Ok(items
        .into_iter()
        .map(|obj| {
            let phase = obj
                .status
                .as_ref()
                .and_then(|s| s.phase.clone())
                .unwrap_or_else(|| "Unknown".to_string());
            InventoryItem {
                kind: "PersistentVolumeClaim".to_string(),
                name: obj.name_any(),
                namespace: object_namespace(&obj),
                extra: Some(phase),
            }
        })
        .collect())
}

async fn list_configmaps(
    state: &ApiState,
    namespace: Option<&str>,
) -> ApiResult<Vec<InventoryItem>> {
    let items = list_objects::<ConfigMap>(state, namespace).await?;
    Ok(items
        .into_iter()
        .map(|obj| {
            let keys = obj.data.as_ref().map(|d| d.len()).unwrap_or(0)
                + obj.binary_data.as_ref().map(|d| d.len()).unwrap_or(0);
            InventoryItem {
                kind: "ConfigMap".to_string(),
                name: obj.name_any(),
                namespace: object_namespace(&obj),
                extra: Some(format!("{keys} keys")),
            }
        })
        .collect())
}

/// Metadata-only listing: Secret `.data` never enters process memory.
async fn list_secrets(state: &ApiState, namespace: Option<&str>) -> ApiResult<Vec<InventoryItem>> {
    let items = list_object_metadata::<Secret>(state, namespace).await?;
    Ok(items.into_iter().map(secret_from_partial_meta).collect())
}

fn secret_from_partial_meta(obj: PartialObjectMeta<Secret>) -> InventoryItem {
    InventoryItem {
        kind: "Secret".to_string(),
        name: obj.name_any(),
        namespace: object_namespace(&obj),
        extra: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;

    #[test]
    fn resolve_kinds_rejects_unknown() {
        let err = resolve_kinds(Some("StatefulSet"));
        assert!(
            matches!(err, Err(ref e) if e.to_string().contains("unsupported inventory kind")),
            "expected unsupported kind error, got {err:?}"
        );
    }

    #[test]
    fn resolve_kinds_accepts_pvc_alias() {
        let kinds = resolve_kinds(Some("PVC"));
        assert_eq!(kinds.ok(), Some(vec!["PersistentVolumeClaim"]));
    }

    #[test]
    fn filter_by_name_is_case_insensitive() {
        let items = vec![
            InventoryItem {
                kind: "Pod".into(),
                name: "web-0".into(),
                namespace: "demo".into(),
                extra: None,
            },
            InventoryItem {
                kind: "Pod".into(),
                name: "db-0".into(),
                namespace: "demo".into(),
                extra: None,
            },
        ];
        let filtered = filter_by_name(items, Some("WEB"));
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].name, "web-0");
    }

    #[test]
    fn secret_row_is_metadata_only() {
        let meta = PartialObjectMeta::<Secret> {
            types: None,
            metadata: ObjectMeta {
                name: Some("db".into()),
                namespace: Some("demo".into()),
                ..Default::default()
            },
            ..Default::default()
        };
        let item = secret_from_partial_meta(meta);
        let json = serde_json::to_string(&item).expect("serialize inventory item");
        assert!(json.contains("\"name\":\"db\""));
        assert!(!json.contains("data"));
        assert!(!json.contains("s3cret"));
        assert!(!json.contains("password"));
        assert_eq!(item.extra, None);
    }
}

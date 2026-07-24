use k8s_openapi::api::core::v1::Namespace;
use kube::api::ListParams;
use kube::Api;
use serde::Serialize;

use crate::error::ApiResult;
use crate::state::ApiState;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NamespaceItem {
    pub name: String,
}

pub async fn list_namespaces(state: &ApiState) -> ApiResult<Vec<NamespaceItem>> {
    let api: Api<Namespace> = Api::all(state.client.clone());
    let list = api.list(&ListParams::default()).await?;
    let mut items: Vec<_> = list
        .items
        .into_iter()
        .filter_map(|ns| ns.metadata.name.map(|name| NamespaceItem { name }))
        .collect();
    items.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(items)
}

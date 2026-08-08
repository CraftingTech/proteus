//! Provision ServiceAccount + ClusterRoleBinding so mover Pods can open repos / patch status.

use std::collections::BTreeMap;

use anyhow::{Context, Result};
use k8s_openapi::api::core::v1::ServiceAccount;
use k8s_openapi::api::rbac::v1::{ClusterRoleBinding, RoleRef, Subject};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
use kube::api::PostParams;
use kube::{Api, Client};
use tracing::info;

pub const MOVER_SA: &str = "proteus-mover";
pub const MOVER_CLUSTER_ROLE: &str = "proteus-mover";

/// Ensure a namespaced SA bound to the narrow `proteus-mover` ClusterRole exists.
pub async fn ensure_mover_identity(client: &Client, namespace: &str) -> Result<()> {
    let sa_api: Api<ServiceAccount> = Api::namespaced(client.clone(), namespace);
    match sa_api.get(MOVER_SA).await {
        Ok(_) => {}
        Err(kube::Error::Api(err)) if err.code == 404 => {
            let sa = ServiceAccount {
                metadata: ObjectMeta {
                    name: Some(MOVER_SA.into()),
                    namespace: Some(namespace.into()),
                    labels: Some(BTreeMap::from([
                        ("app.kubernetes.io/name".into(), "proteus".into()),
                        ("app.kubernetes.io/component".into(), "mover".into()),
                    ])),
                    ..Default::default()
                },
                ..Default::default()
            };
            sa_api
                .create(&PostParams::default(), &sa)
                .await
                .with_context(|| format!("create ServiceAccount {namespace}/{MOVER_SA}"))?;
            info!(%namespace, "created mover ServiceAccount");
        }
        Err(err) => {
            return Err(err).with_context(|| format!("get ServiceAccount {namespace}/{MOVER_SA}"));
        }
    }

    let crb_name = format!("proteus-mover-{namespace}");
    let crb_api: Api<ClusterRoleBinding> = Api::all(client.clone());
    match crb_api.get(&crb_name).await {
        Ok(_) => {}
        Err(kube::Error::Api(err)) if err.code == 404 => {
            let crb = ClusterRoleBinding {
                metadata: ObjectMeta {
                    name: Some(crb_name.clone()),
                    labels: Some(BTreeMap::from([
                        ("app.kubernetes.io/name".into(), "proteus".into()),
                        ("app.kubernetes.io/component".into(), "mover".into()),
                    ])),
                    ..Default::default()
                },
                role_ref: RoleRef {
                    api_group: "rbac.authorization.k8s.io".into(),
                    kind: "ClusterRole".into(),
                    name: MOVER_CLUSTER_ROLE.into(),
                },
                subjects: Some(vec![Subject {
                    kind: "ServiceAccount".into(),
                    name: MOVER_SA.into(),
                    namespace: Some(namespace.into()),
                    ..Default::default()
                }]),
            };
            crb_api
                .create(&PostParams::default(), &crb)
                .await
                .with_context(|| format!("create ClusterRoleBinding {crb_name}"))?;
            info!(%crb_name, "created mover ClusterRoleBinding");
        }
        Err(err) => {
            return Err(err).with_context(|| format!("get ClusterRoleBinding {crb_name}"));
        }
    }
    Ok(())
}

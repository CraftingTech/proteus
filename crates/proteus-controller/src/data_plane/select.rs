use std::collections::BTreeSet;

use k8s_openapi::api::core::v1::{PersistentVolume, PersistentVolumeClaim, Pod};
use kube::api::ListParams;
use kube::{Api, Client};
use proteus_crd::DataPlane;

use crate::agent::AGENT_READY_LABEL;

/// Whether the repository backend can be opened from an agent/mover Pod.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RepositoryKind {
    /// S3-compatible (or other remote) — agent plane eligible.
    Remote,
    /// Controller-local filesystem (emptyDir) — force exec.
    Local,
}

/// Outcome of plane selection for one Backup/Restore run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlaneChoice {
    Exec { reason: String },
    Agent { node: String },
}

impl PlaneChoice {
    pub fn data_plane(&self) -> DataPlane {
        match self {
            Self::Exec { .. } => DataPlane::Exec,
            Self::Agent { .. } => DataPlane::Agent,
        }
    }

    pub fn assigned_node(&self) -> Option<&str> {
        match self {
            Self::Agent { node } => Some(node.as_str()),
            Self::Exec { .. } => None,
        }
    }
}

/// Pick exec vs agent for the given PVC set and repository kind.
pub async fn select_plane(
    client: &Client,
    repo: RepositoryKind,
    pvc_namespace: &str,
    pvc_names: &[String],
) -> Result<PlaneChoice, String> {
    if matches!(repo, RepositoryKind::Local) {
        return Ok(PlaneChoice::Exec {
            reason: "local repository is only reachable from the controller; using exec data plane"
                .into(),
        });
    }

    if pvc_names.is_empty() {
        return Ok(PlaneChoice::Exec {
            reason: "no PVCs to move; using exec data plane".into(),
        });
    }

    let ready_nodes = list_ready_agent_nodes(client).await?;
    if ready_nodes.is_empty() {
        return Ok(PlaneChoice::Exec {
            reason: "no Ready proteus-node-agent Pods; using exec data plane".into(),
        });
    }

    let mut nodes = BTreeSet::new();
    for pvc in pvc_names {
        let node = resolve_pvc_node(client, pvc_namespace, pvc).await?;
        nodes.insert(node);
    }

    if nodes.len() != 1 {
        return Ok(PlaneChoice::Exec {
            reason: format!(
                "PVCs span {} nodes {:?}; agent plane requires a single node (using exec)",
                nodes.len(),
                nodes
            ),
        });
    }

    let node = nodes.into_iter().next().unwrap_or_default();
    if !ready_nodes.contains(&node) {
        return Ok(PlaneChoice::Exec {
            reason: format!(
                "no Ready agent on node '{node}' (agents on {:?}); using exec data plane",
                ready_nodes
            ),
        });
    }

    Ok(PlaneChoice::Agent { node })
}

/// Nodes that currently have a DaemonSet agent Pod labeled Ready.
pub async fn list_ready_agent_nodes(client: &Client) -> Result<BTreeSet<String>, String> {
    let pods: Api<Pod> = Api::all(client.clone());
    let lp = ListParams::default().labels(&format!("{AGENT_READY_LABEL}=true"));
    let list = pods
        .list(&lp)
        .await
        .map_err(|err| format!("list Ready agent Pods: {err}"))?;

    let mut nodes = BTreeSet::new();
    for pod in list {
        let Some(spec) = pod.spec.as_ref() else {
            continue;
        };
        if let Some(node) = spec.node_name.as_ref() {
            if !node.is_empty() {
                nodes.insert(node.clone());
            }
        }
    }
    Ok(nodes)
}

/// Best-effort node that currently holds `pvc_name` data.
///
/// Prefers a running Pod that mounts the PVC; falls back to the bound PV's
/// `nodeAffinity` required terms when present.
pub async fn resolve_pvc_node(
    client: &Client,
    namespace: &str,
    pvc_name: &str,
) -> Result<String, String> {
    if let Some(node) = node_from_consumer_pod(client, namespace, pvc_name).await? {
        return Ok(node);
    }
    if let Some(node) = node_from_bound_pv(client, namespace, pvc_name).await? {
        return Ok(node);
    }
    Err(format!(
        "PVC '{namespace}/{pvc_name}' has no consumer Pod with nodeName and no PV nodeAffinity; \
         cannot assign agent plane"
    ))
}

async fn node_from_consumer_pod(
    client: &Client,
    namespace: &str,
    pvc_name: &str,
) -> Result<Option<String>, String> {
    let pods: Api<Pod> = Api::namespaced(client.clone(), namespace);
    let list = pods
        .list(&ListParams::default())
        .await
        .map_err(|err| format!("list Pods in '{namespace}': {err}"))?;

    for pod in list {
        let Some(spec) = pod.spec.as_ref() else {
            continue;
        };
        let uses_pvc = spec.volumes.iter().flatten().any(|vol| {
            vol.persistent_volume_claim
                .as_ref()
                .is_some_and(|claim| claim.claim_name == pvc_name)
        });
        if !uses_pvc {
            continue;
        }
        if let Some(node) = spec.node_name.as_ref() {
            if !node.is_empty() {
                return Ok(Some(node.clone()));
            }
        }
    }
    Ok(None)
}

async fn node_from_bound_pv(
    client: &Client,
    namespace: &str,
    pvc_name: &str,
) -> Result<Option<String>, String> {
    let pvcs: Api<PersistentVolumeClaim> = Api::namespaced(client.clone(), namespace);
    let pvc = pvcs
        .get(pvc_name)
        .await
        .map_err(|err| format!("get PVC '{namespace}/{pvc_name}': {err}"))?;
    let Some(volume_name) = pvc.spec.as_ref().and_then(|s| s.volume_name.clone()) else {
        return Ok(None);
    };

    let pvs: Api<PersistentVolume> = Api::all(client.clone());
    let pv = pvs
        .get(&volume_name)
        .await
        .map_err(|err| format!("get PV '{volume_name}': {err}"))?;

    Ok(node_from_pv_affinity(&pv))
}

fn node_from_pv_affinity(pv: &PersistentVolume) -> Option<String> {
    let terms = pv
        .spec
        .as_ref()?
        .node_affinity
        .as_ref()?
        .required
        .as_ref()?
        .node_selector_terms
        .as_slice();

    for term in terms {
        for expr in term.match_expressions.iter().flatten() {
            if expr.key == "kubernetes.io/hostname" && expr.operator == "In" {
                if let Some(values) = expr.values.as_ref() {
                    if let Some(node) = values.first() {
                        if !node.is_empty() {
                            return Some(node.clone());
                        }
                    }
                }
            }
        }
        for field in term.match_fields.iter().flatten() {
            if field.key == "metadata.name" && field.operator == "In" {
                if let Some(values) = field.values.as_ref() {
                    if let Some(node) = values.first() {
                        if !node.is_empty() {
                            return Some(node.clone());
                        }
                    }
                }
            }
        }
    }
    None
}

/// Pure helper for unit tests: given known Ready nodes + PVC→node map, choose a plane.
#[cfg(test)]
pub fn select_plane_from_maps(
    repo: RepositoryKind,
    ready_nodes: &BTreeSet<String>,
    pvc_nodes: &[String],
) -> PlaneChoice {
    if matches!(repo, RepositoryKind::Local) {
        return PlaneChoice::Exec {
            reason: "local repository".into(),
        };
    }
    if pvc_nodes.is_empty() {
        return PlaneChoice::Exec {
            reason: "no PVCs".into(),
        };
    }
    if ready_nodes.is_empty() {
        return PlaneChoice::Exec {
            reason: "no Ready agents".into(),
        };
    }
    let unique: BTreeSet<_> = pvc_nodes.iter().cloned().collect();
    if unique.len() != 1 {
        return PlaneChoice::Exec {
            reason: "multi-node PVCs".into(),
        };
    }
    let node = unique.into_iter().next().unwrap_or_default();
    if !ready_nodes.contains(&node) {
        return PlaneChoice::Exec {
            reason: format!("no agent on {node}"),
        };
    }
    PlaneChoice::Agent { node }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_repo_forces_exec() {
        let ready = BTreeSet::from(["node-a".into()]);
        let choice = select_plane_from_maps(RepositoryKind::Local, &ready, &["node-a".into()]);
        assert!(matches!(choice, PlaneChoice::Exec { .. }));
        assert_eq!(choice.data_plane(), DataPlane::Exec);
    }

    #[test]
    fn ready_agent_on_pvc_node_selects_agent() {
        let ready = BTreeSet::from(["node-a".into(), "node-b".into()]);
        let choice = select_plane_from_maps(RepositoryKind::Remote, &ready, &["node-a".into()]);
        assert_eq!(
            choice,
            PlaneChoice::Agent {
                node: "node-a".into()
            }
        );
        assert_eq!(choice.assigned_node(), Some("node-a"));
    }

    #[test]
    fn missing_agent_forces_exec() {
        let ready = BTreeSet::from(["node-b".into()]);
        let choice = select_plane_from_maps(RepositoryKind::Remote, &ready, &["node-a".into()]);
        assert!(matches!(choice, PlaneChoice::Exec { .. }));
    }

    #[test]
    fn multi_node_pvcs_force_exec() {
        let ready = BTreeSet::from(["node-a".into(), "node-b".into()]);
        let choice = select_plane_from_maps(
            RepositoryKind::Remote,
            &ready,
            &["node-a".into(), "node-b".into()],
        );
        assert!(matches!(choice, PlaneChoice::Exec { .. }));
    }
}

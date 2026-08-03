use kube::{Api, Client, ResourceExt};
use proteus_crd::{
    BackupPolicyPhase, ProteusBackup, ProteusBackupPolicy, ProteusBackupSpec, RetentionPolicy,
};

/// Resolved backup recipe used by the data path (policy or inline).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BackupRecipe {
    pub repository_ref: String,
    pub repository_namespace: Option<String>,
    pub target_namespace: String,
    pub pvc_names: Vec<String>,
    pub retention: RetentionPolicy,
    pub include_volumes: bool,
    pub include_cluster_resources: bool,
}

impl BackupRecipe {
    pub fn from_inline(spec: &ProteusBackupSpec) -> Self {
        Self {
            repository_ref: spec.repository_ref.clone(),
            repository_namespace: spec.repository_namespace.clone(),
            target_namespace: spec.target_namespace.clone(),
            pvc_names: spec.pvc_names.clone(),
            retention: spec.retention.clone(),
            include_volumes: spec.include_volumes,
            include_cluster_resources: spec.include_cluster_resources,
        }
    }

    pub fn from_policy(policy: &ProteusBackupPolicy) -> Self {
        Self {
            repository_ref: policy.spec.repository_ref.clone(),
            repository_namespace: policy.spec.repository_namespace.clone(),
            target_namespace: policy.spec.target_namespace.clone(),
            pvc_names: policy.spec.pvc_names.clone(),
            retention: policy.spec.retention.clone(),
            include_volumes: policy.spec.include_volumes,
            include_cluster_resources: policy.spec.include_cluster_resources,
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.repository_ref.is_empty() {
            return Err("repositoryRef must not be empty".to_string());
        }
        if self.target_namespace.is_empty() {
            return Err("targetNamespace must not be empty".to_string());
        }
        if self.pvc_names.is_empty() {
            return Err("pvcNames must contain at least one PVC name".to_string());
        }
        if self.retention.keep_last == 0 {
            return Err("retention.keepLast must be >= 1".to_string());
        }
        Ok(())
    }
}

pub fn validate_policy_spec(policy: &ProteusBackupPolicy) -> Result<(), String> {
    BackupRecipe::from_policy(policy).validate()?;
    if let Some(schedule) = policy
        .spec
        .schedule
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        proteus_core::validate_schedule(schedule)?;
    }
    Ok(())
}

/// Why a run cannot start yet vs permanently.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ResolveRecipeError {
    /// Policy exists but is not Ready yet — requeue the run.
    NotReady(String),
    /// Missing/Invalid policy or invalid recipe — fail the run.
    Failed(String),
}

/// Load recipe from `policyRef` (must be Ready) or inline Backup spec.
pub async fn resolve_recipe(
    client: &Client,
    backup: &ProteusBackup,
    backup_namespace: &str,
) -> Result<BackupRecipe, ResolveRecipeError> {
    let Some(policy_ref) = backup
        .spec
        .policy_ref
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    else {
        let recipe = BackupRecipe::from_inline(&backup.spec);
        return recipe
            .validate()
            .map(|()| recipe)
            .map_err(ResolveRecipeError::Failed);
    };

    let policy_ns = policy_namespace(backup, backup_namespace);
    let api: Api<ProteusBackupPolicy> = Api::namespaced(client.clone(), policy_ns);
    let policy = match api.get(policy_ref).await {
        Ok(p) => p,
        Err(err) => {
            return Err(ResolveRecipeError::Failed(format!(
                "policyRef '{policy_ref}' in namespace '{policy_ns}': {err}"
            )));
        }
    };

    match policy.status.as_ref().and_then(|s| s.phase.as_ref()) {
        Some(BackupPolicyPhase::Ready) => {}
        Some(BackupPolicyPhase::Invalid) => {
            let message = policy
                .status
                .as_ref()
                .and_then(|s| s.message.as_deref())
                .unwrap_or("policy is Invalid");
            return Err(ResolveRecipeError::Failed(format!(
                "policyRef '{}' in namespace '{}' is Invalid: {message}",
                policy.name_any(),
                policy_ns
            )));
        }
        None => {
            return Err(ResolveRecipeError::NotReady(format!(
                "waiting for policyRef '{}' in namespace '{}' to become Ready",
                policy.name_any(),
                policy_ns
            )));
        }
    }

    let recipe = BackupRecipe::from_policy(&policy);
    recipe
        .validate()
        .map(|()| recipe)
        .map_err(ResolveRecipeError::Failed)
}

/// Recipe for restore / GC: prefer live policy fields, else stamped inline.
///
/// Does not require Ready — historical runs must still find their repository.
pub async fn load_recipe(
    client: &Client,
    backup: &ProteusBackup,
    backup_namespace: &str,
) -> Result<BackupRecipe, String> {
    let Some(policy_ref) = backup
        .spec
        .policy_ref
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    else {
        let recipe = BackupRecipe::from_inline(&backup.spec);
        recipe.validate()?;
        return Ok(recipe);
    };

    let policy_ns = policy_namespace(backup, backup_namespace);
    let api: Api<ProteusBackupPolicy> = Api::namespaced(client.clone(), policy_ns);
    match api.get(policy_ref).await {
        Ok(policy) => {
            let recipe = BackupRecipe::from_policy(&policy);
            recipe.validate()?;
            Ok(recipe)
        }
        Err(err) => {
            let stamped = BackupRecipe::from_inline(&backup.spec);
            if stamped.validate().is_ok() {
                return Ok(stamped);
            }
            Err(format!(
                "policyRef '{policy_ref}' in namespace '{policy_ns}': {err}"
            ))
        }
    }
}

fn policy_namespace<'a>(backup: &'a ProteusBackup, backup_namespace: &'a str) -> &'a str {
    backup
        .spec
        .policy_namespace
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(backup_namespace)
}

#[cfg(test)]
mod tests {
    use super::*;
    use proteus_crd::{ProteusBackupPolicySpec, RetentionPolicy};

    fn inline_backup(pvc_names: Vec<String>) -> ProteusBackup {
        ProteusBackup::new(
            "run",
            ProteusBackupSpec {
                policy_ref: None,
                policy_namespace: None,
                repository_ref: "repo".into(),
                repository_namespace: None,
                target_namespace: "default".into(),
                pvc_names,
                label_selector: None,
                schedule: None,
                retention: RetentionPolicy::default(),
                include_volumes: true,
                include_cluster_resources: false,
            },
        )
    }

    fn policy(pvc_names: Vec<String>) -> ProteusBackupPolicy {
        ProteusBackupPolicy::new(
            "nightly",
            ProteusBackupPolicySpec {
                repository_ref: "repo".into(),
                repository_namespace: None,
                target_namespace: "workloads".into(),
                pvc_names,
                label_selector: None,
                schedule: None,
                paused: false,
                retention: RetentionPolicy {
                    keep_last: 3,
                    max_age_days: None,
                },
                include_volumes: true,
                include_cluster_resources: false,
            },
        )
    }

    #[test]
    fn inline_recipe_validates() {
        let recipe = BackupRecipe::from_inline(&inline_backup(vec!["data".into()]).spec);
        assert!(recipe.validate().is_ok());
    }

    #[test]
    fn inline_recipe_rejects_empty_pvcs() {
        let recipe = BackupRecipe::from_inline(&inline_backup(vec![]).spec);
        assert!(recipe.validate().is_err());
    }

    #[test]
    fn policy_recipe_maps_fields() {
        let recipe = BackupRecipe::from_policy(&policy(vec!["pvc-a".into()]));
        assert_eq!(recipe.target_namespace, "workloads");
        assert_eq!(recipe.pvc_names, vec!["pvc-a".to_string()]);
        assert_eq!(recipe.retention.keep_last, 3);
    }

    #[test]
    fn validate_policy_spec_rejects_empty_repo() {
        let mut p = policy(vec!["data".into()]);
        p.spec.repository_ref.clear();
        assert!(validate_policy_spec(&p).is_err());
    }
}

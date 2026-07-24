use kube::CustomResource;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(CustomResource, Deserialize, Serialize, Clone, Debug, JsonSchema)]
#[kube(
    group = "proteus.io",
    version = "v1alpha1",
    kind = "ProteusRepository",
    plural = "proteusrepositories",
    shortname = "prepo",
    status = "ProteusRepositoryStatus",
    namespaced,
    printcolumn = r#"{"name":"Backend","type":"string","jsonPath":".spec.backend.type"}"#,
    printcolumn = r#"{"name":"Phase","type":"string","jsonPath":".status.phase"}"#,
    printcolumn = r#"{"name":"Age","type":"date","jsonPath":".metadata.creationTimestamp"}"#
)]
#[serde(rename_all = "camelCase")]
pub struct ProteusRepositorySpec {
    pub backend: RepositoryBackend,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default)]
    pub encryption_enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub encryption_secret_ref: Option<String>,
}

/// Storage backend as a tagged union (`type: s3|local` + flat fields).
///
/// Custom `JsonSchema` avoids kube's panic when merging conflicting `type` enums
/// across generated `oneOf` arms.
#[derive(Deserialize, Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase", tag = "type")]
pub enum RepositoryBackend {
    #[serde(rename = "s3")]
    S3(S3BackendSpec),
    #[serde(rename = "local")]
    Local(LocalBackendSpec),
}

impl JsonSchema for RepositoryBackend {
    fn schema_name() -> String {
        "RepositoryBackend".to_owned()
    }

    fn json_schema(_gen: &mut schemars::gen::SchemaGenerator) -> schemars::schema::Schema {
        let value = serde_json::json!({
            "oneOf": [
                {
                    "type": "object",
                    "required": ["type", "bucket", "credentialsSecretRef"],
                    "properties": {
                        "type": { "type": "string" },
                        "bucket": { "type": "string" },
                        "prefix": { "type": "string" },
                        "endpoint": { "type": "string" },
                        "region": { "type": "string" },
                        "credentialsSecretRef": { "type": "string" },
                        "forcePathStyle": { "type": "boolean" }
                    }
                },
                {
                    "type": "object",
                    "required": ["type", "path"],
                    "properties": {
                        "type": { "type": "string" },
                        "path": { "type": "string" }
                    }
                }
            ],
            "x-kubernetes-validations": [
                {
                    "rule": "self.type in ['s3', 'local']",
                    "message": "backend.type must be s3 or local"
                },
                {
                    "rule": "self.type != 's3' || (has(self.bucket) && has(self.credentialsSecretRef))",
                    "message": "s3 backend requires bucket and credentialsSecretRef"
                },
                {
                    "rule": "self.type != 'local' || has(self.path)",
                    "message": "local backend requires path"
                }
            ]
        });
        match serde_json::from_value(value) {
            Ok(schema) => schema,
            Err(_) => schemars::schema::Schema::Bool(false),
        }
    }
}

#[derive(Deserialize, Serialize, Clone, Debug, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct S3BackendSpec {
    pub bucket: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prefix: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub region: Option<String>,
    pub credentials_secret_ref: String,
    #[serde(default)]
    pub force_path_style: bool,
}

#[derive(Deserialize, Serialize, Clone, Debug, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct LocalBackendSpec {
    pub path: String,
}

#[derive(Deserialize, Serialize, Clone, Debug, JsonSchema, Default)]
#[serde(rename_all = "camelCase")]
pub struct ProteusRepositoryStatus {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase: Option<RepositoryPhase>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub object_count: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bytes_stored: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_checked_at: Option<String>,
}

#[derive(Deserialize, Serialize, Clone, Debug, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "PascalCase")]
pub enum RepositoryPhase {
    Pending,
    Ready,
    Failed,
    Terminating,
}

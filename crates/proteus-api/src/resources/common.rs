use serde::Serialize;
use serde_json::Value;

use crate::error::{ApiError, ApiResult};

pub(crate) const DEFAULT_REPO_NAMESPACE: &str = "proteus-system";

pub(crate) fn phase_label<T: Serialize>(phase: &T) -> Option<String> {
    match serde_json::to_value(phase) {
        Ok(Value::String(label)) => Some(label),
        _ => None,
    }
}

pub(crate) fn require_non_empty(field: &str, value: Option<&str>) -> ApiResult<String> {
    match value.map(str::trim).filter(|s| !s.is_empty()) {
        Some(v) => Ok(v.to_string()),
        None => Err(ApiError::BadRequest(format!("{field} is required"))),
    }
}

pub(crate) fn resolve_namespace(requested: Option<&str>) -> ApiResult<String> {
    match requested.map(str::trim).filter(|s| !s.is_empty()) {
        Some(ns) => Ok(ns.to_string()),
        None => Ok(DEFAULT_REPO_NAMESPACE.to_string()),
    }
}

pub(crate) fn optional_trimmed(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

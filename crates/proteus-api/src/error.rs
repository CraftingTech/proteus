use std::net::SocketAddr;

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use thiserror::Error;

pub type ApiResult<T> = Result<T, ApiError>;

#[derive(Debug, Error)]
pub enum ApiError {
    #[error("failed to bind API listener on {addr}: {source}")]
    Bind {
        addr: SocketAddr,
        #[source]
        source: std::io::Error,
    },

    #[error("API server error: {source}")]
    Serve {
        #[source]
        source: std::io::Error,
    },

    #[error("bad request: {0}")]
    BadRequest(String),

    #[error("not ready")]
    NotReady,

    #[error("kubernetes error: {0}")]
    Kubernetes(#[from] kube::Error),

    #[error("internal error: {0}")]
    Internal(String),
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = match &self {
            ApiError::BadRequest(_) => StatusCode::BAD_REQUEST,
            ApiError::NotReady => StatusCode::SERVICE_UNAVAILABLE,
            ApiError::Kubernetes(_) => StatusCode::BAD_GATEWAY,
            ApiError::Internal(_) | ApiError::Bind { .. } | ApiError::Serve { .. } => {
                StatusCode::INTERNAL_SERVER_ERROR
            }
        };
        let body = serde_json::json!({ "error": self.to_string() });
        (status, axum::Json(body)).into_response()
    }
}

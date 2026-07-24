use thiserror::Error;

#[derive(Debug, Error)]
pub enum ControllerError {
    #[error("kubernetes error: {0}")]
    Kubernetes(#[from] kube::Error),

    #[error("invalid resource spec: {0}")]
    InvalidSpec(String),

    #[error("storage error: {0}")]
    Storage(#[from] proteus_core::CoreError),
}

pub type ControllerResult<T> = Result<T, ControllerError>;

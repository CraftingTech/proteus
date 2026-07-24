use std::path::PathBuf;

use thiserror::Error;

pub type CoreResult<T> = Result<T, CoreError>;

#[derive(Debug, Error)]
pub enum CoreError {
    #[error("local storage I/O error at {path}: {source}")]
    LocalIo {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("object not found: {0}")]
    NotFound(String),

    #[error("cryptographic operation failed: {0}")]
    Crypto(String),

    #[error("invalid encryption key: {0}")]
    InvalidKey(String),

    #[error("S3 backend not implemented: {0}")]
    S3NotImplemented(String),

    #[error("S3 error: {0}")]
    S3(String),

    #[error("invalid argument: {0}")]
    InvalidArgument(String),
}

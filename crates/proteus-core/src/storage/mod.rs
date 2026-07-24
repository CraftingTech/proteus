mod local;
mod s3;
mod traits;

pub use local::LocalBackend;
pub use s3::{S3Backend, S3Config};
pub use traits::{ObjectStore, PutOptions, StoredObject};

use async_trait::async_trait;
use bytes::Bytes;

use super::traits::{ObjectStore, PutOptions, StoredObject};
use crate::error::{CoreError, CoreResult};
use crate::hash::ContentId;

#[derive(Clone, Debug)]
pub struct S3Config {
    pub bucket: String,
    pub prefix: Option<String>,
    pub endpoint: Option<String>,
    pub region: Option<String>,
    pub force_path_style: bool,
}

#[derive(Clone, Debug)]
pub struct S3Backend {
    config: S3Config,
}

impl S3Backend {
    pub fn new(config: S3Config) -> Self {
        Self { config }
    }

    pub fn bucket(&self) -> &str {
        &self.config.bucket
    }
}

#[async_trait]
impl ObjectStore for S3Backend {
    async fn put(
        &self,
        _id: &ContentId,
        _data: Bytes,
        _opts: PutOptions,
    ) -> CoreResult<StoredObject> {
        Err(CoreError::S3NotImplemented(format!(
            "put into bucket {}",
            self.config.bucket
        )))
    }

    async fn get(&self, id: &ContentId) -> CoreResult<Bytes> {
        Err(CoreError::S3NotImplemented(format!(
            "get {id} from bucket {}",
            self.config.bucket
        )))
    }

    async fn exists(&self, id: &ContentId) -> CoreResult<bool> {
        Err(CoreError::S3NotImplemented(format!(
            "exists {id} in bucket {}",
            self.config.bucket
        )))
    }

    async fn delete(&self, id: &ContentId) -> CoreResult<()> {
        Err(CoreError::S3NotImplemented(format!(
            "delete {id} from bucket {}",
            self.config.bucket
        )))
    }
}

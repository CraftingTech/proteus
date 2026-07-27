use async_trait::async_trait;
use bytes::Bytes;

use crate::error::CoreResult;
use crate::hash::ContentId;

#[derive(Clone, Copy, Debug, Default)]
pub struct PutOptions {
    pub skip_if_exists: bool,
}

#[derive(Clone, Debug)]
pub struct StoredObject {
    pub id: ContentId,
    pub size: u64,
    pub deduplicated: bool,
}

#[async_trait]
pub trait ObjectStore: Send + Sync {
    async fn put(&self, id: &ContentId, data: Bytes, opts: PutOptions) -> CoreResult<StoredObject>;

    async fn get(&self, id: &ContentId) -> CoreResult<Bytes>;

    async fn exists(&self, id: &ContentId) -> CoreResult<bool>;

    async fn delete(&self, id: &ContentId) -> CoreResult<()>;

    /// List every content id currently stored (for GC). Order is undefined.
    async fn list_ids(&self) -> CoreResult<Vec<ContentId>>;
}

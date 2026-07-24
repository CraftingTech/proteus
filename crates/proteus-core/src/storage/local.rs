use std::path::{Path, PathBuf};

use async_trait::async_trait;
use bytes::Bytes;
use tokio::fs;
use tracing::debug;

use super::traits::{ObjectStore, PutOptions, StoredObject};
use crate::error::{CoreError, CoreResult};
use crate::hash::ContentId;

/// Layout: `{root}/{id[0..2]}/{id}.blob`
#[derive(Clone, Debug)]
pub struct LocalBackend {
    root: PathBuf,
}

impl LocalBackend {
    pub async fn open(root: impl AsRef<Path>) -> CoreResult<Self> {
        let root = root.as_ref().to_path_buf();
        fs::create_dir_all(&root)
            .await
            .map_err(|source| CoreError::LocalIo {
                path: root.clone(),
                source,
            })?;
        Ok(Self { root })
    }

    fn object_path(&self, id: &ContentId) -> PathBuf {
        let hex = id.to_hex();
        let (prefix, _) = hex.split_at(2);
        self.root.join(prefix).join(format!("{hex}.blob"))
    }
}

#[async_trait]
impl ObjectStore for LocalBackend {
    async fn put(&self, id: &ContentId, data: Bytes, opts: PutOptions) -> CoreResult<StoredObject> {
        let path = self.object_path(id);

        if opts.skip_if_exists {
            match fs::try_exists(&path).await {
                Ok(true) => {
                    let size = fs::metadata(&path)
                        .await
                        .map_err(|source| CoreError::LocalIo {
                            path: path.clone(),
                            source,
                        })?
                        .len();
                    debug!(%id, "deduplicated local put");
                    return Ok(StoredObject {
                        id: id.clone(),
                        size,
                        deduplicated: true,
                    });
                }
                Ok(false) => {}
                Err(source) => {
                    return Err(CoreError::LocalIo {
                        path: path.clone(),
                        source,
                    });
                }
            }
        }

        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .await
                .map_err(|source| CoreError::LocalIo {
                    path: parent.to_path_buf(),
                    source,
                })?;
        }

        let tmp = path.with_extension("blob.tmp");
        fs::write(&tmp, &data)
            .await
            .map_err(|source| CoreError::LocalIo {
                path: tmp.clone(),
                source,
            })?;
        fs::rename(&tmp, &path)
            .await
            .map_err(|source| CoreError::LocalIo {
                path: path.clone(),
                source,
            })?;

        Ok(StoredObject {
            id: id.clone(),
            size: data.len() as u64,
            deduplicated: false,
        })
    }

    async fn get(&self, id: &ContentId) -> CoreResult<Bytes> {
        let path = self.object_path(id);
        let data = fs::read(&path).await.map_err(|source| {
            if source.kind() == std::io::ErrorKind::NotFound {
                CoreError::NotFound(id.to_hex())
            } else {
                CoreError::LocalIo {
                    path: path.clone(),
                    source,
                }
            }
        })?;
        Ok(Bytes::from(data))
    }

    async fn exists(&self, id: &ContentId) -> CoreResult<bool> {
        let path = self.object_path(id);
        fs::try_exists(&path)
            .await
            .map_err(|source| CoreError::LocalIo { path, source })
    }

    async fn delete(&self, id: &ContentId) -> CoreResult<()> {
        let path = self.object_path(id);
        match fs::remove_file(&path).await {
            Ok(()) => Ok(()),
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(source) => Err(CoreError::LocalIo { path, source }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hash::hash_bytes;

    #[tokio::test]
    async fn put_get_round_trip() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = LocalBackend::open(dir.path()).await.expect("open");
        let data = Bytes::from_static(b"hello-cas");
        let id = hash_bytes(&data);
        store
            .put(&id, data.clone(), PutOptions::default())
            .await
            .expect("put");
        let got = store.get(&id).await.expect("get");
        assert_eq!(got, data);
    }
}

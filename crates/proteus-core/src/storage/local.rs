use std::path::PathBuf;

use async_trait::async_trait;
use bytes::Bytes;
use tokio::fs;
use tracing::debug;

use super::traits::{ObjectStore, PutOptions, StoredObject};
use crate::error::{CoreError, CoreResult};
use crate::hash::ContentId;

/// Expand `~` / `~/…` using `$HOME` (or `$USERPROFILE` on Windows).
pub fn expand_local_path(path: impl AsRef<str>) -> PathBuf {
    let path = path.as_ref().trim();
    if path.is_empty() {
        return PathBuf::new();
    }
    if path == "~" {
        return home_dir().unwrap_or_else(|| PathBuf::from("/"));
    }
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = home_dir() {
            return home.join(rest);
        }
    }
    PathBuf::from(path)
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

/// Layout: `{root}/{id[0..2]}/{id}.blob`
#[derive(Clone, Debug)]
pub struct LocalBackend {
    root: PathBuf,
}

impl LocalBackend {
    pub async fn open(root: impl AsRef<str>) -> CoreResult<Self> {
        let root = expand_local_path(root.as_ref());
        fs::create_dir_all(&root)
            .await
            .map_err(|source| CoreError::LocalIo {
                path: root.clone(),
                source,
            })?;
        Ok(Self { root })
    }

    /// Ensure `path` exists (creating parents) and is writable. Expands `~`.
    pub async fn probe(path: impl AsRef<str>) -> CoreResult<PathBuf> {
        let root = expand_local_path(path.as_ref());
        if root.as_os_str().is_empty() {
            return Err(CoreError::InvalidArgument(
                "local backend path must not be empty".into(),
            ));
        }
        fs::create_dir_all(&root)
            .await
            .map_err(|source| CoreError::LocalIo {
                path: root.clone(),
                source,
            })?;

        let probe = root.join(".proteus-write-probe");
        fs::write(&probe, b"ok")
            .await
            .map_err(|source| CoreError::LocalIo {
                path: probe.clone(),
                source,
            })?;
        fs::remove_file(&probe)
            .await
            .map_err(|source| CoreError::LocalIo {
                path: probe,
                source,
            })?;
        Ok(root)
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

    async fn list_ids(&self) -> CoreResult<Vec<ContentId>> {
        let mut out = Vec::new();
        let mut stack = vec![self.root.clone()];
        while let Some(dir) = stack.pop() {
            let mut entries = fs::read_dir(&dir)
                .await
                .map_err(|source| CoreError::LocalIo {
                    path: dir.clone(),
                    source,
                })?;
            loop {
                let entry = entries
                    .next_entry()
                    .await
                    .map_err(|source| CoreError::LocalIo {
                        path: dir.clone(),
                        source,
                    })?;
                let Some(entry) = entry else {
                    break;
                };
                let path = entry.path();
                let file_type = entry
                    .file_type()
                    .await
                    .map_err(|source| CoreError::LocalIo {
                        path: path.clone(),
                        source,
                    })?;
                if file_type.is_dir() {
                    stack.push(path);
                    continue;
                }
                if !file_type.is_file() {
                    continue;
                }
                let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
                    continue;
                };
                let Some(hex) = name.strip_suffix(".blob") else {
                    continue;
                };
                if let Ok(id) = ContentId::from_hex(hex) {
                    out.push(id);
                }
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hash::hash_bytes;

    #[tokio::test]
    async fn put_get_round_trip() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = LocalBackend::open(dir.path().to_str().expect("utf8"))
            .await
            .expect("open");
        let data = Bytes::from_static(b"hello-cas");
        let id = hash_bytes(&data);
        store
            .put(&id, data.clone(), PutOptions::default())
            .await
            .expect("put");
        let got = store.get(&id).await.expect("get");
        assert_eq!(got, data);
    }

    #[tokio::test]
    async fn probe_accepts_writable_path() {
        let dir = tempfile::tempdir().expect("tempdir");
        LocalBackend::probe(dir.path().to_str().expect("utf8"))
            .await
            .expect("probe");
    }

    #[tokio::test]
    async fn probe_creates_missing_dir() {
        let dir = tempfile::tempdir().expect("tempdir");
        let nested = dir.path().join("nested").join("repo");
        let resolved = LocalBackend::probe(nested.to_str().expect("utf8"))
            .await
            .expect("probe creates");
        assert!(resolved.is_dir());
    }

    #[tokio::test]
    async fn expand_tilde() {
        std::env::set_var("HOME", "/tmp/proteus-home-test");
        assert_eq!(
            expand_local_path("~/backup"),
            PathBuf::from("/tmp/proteus-home-test/backup")
        );
        assert_eq!(
            expand_local_path("~"),
            PathBuf::from("/tmp/proteus-home-test")
        );
    }

    #[tokio::test]
    async fn probe_rejects_impossible_path() {
        let path = "/proc/proteus-no-such-dir/nested";
        let err = LocalBackend::probe(path).await.expect_err("should fail");
        assert!(matches!(err, CoreError::LocalIo { .. }));
    }
}

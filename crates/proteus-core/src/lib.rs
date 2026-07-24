//! CAS primitives for Proteus: chunking, BLAKE3 ids, AES-GCM, storage backends.

#![deny(clippy::unwrap_used)]
#![deny(clippy::expect_used)]
#![deny(clippy::panic)]

pub mod chunking;
pub mod crypto;
pub mod error;
pub mod hash;
pub mod storage;

pub use chunking::{Chunk, Chunker, DEFAULT_CHUNK_SIZE};
pub use crypto::{decrypt, encrypt, Ciphertext, EncryptionKey};
pub use error::{CoreError, CoreResult};
pub use hash::{hash_bytes, ContentId};
pub use storage::{LocalBackend, ObjectStore, PutOptions, S3Backend, S3Config, StoredObject};

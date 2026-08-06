//! CAS primitives for Proteus: chunking, BLAKE3 ids, AES-GCM, storage backends.

#![deny(clippy::unwrap_used)]
#![deny(clippy::expect_used)]
#![deny(clippy::panic)]

pub mod backup;
pub mod chunking;
pub mod crypto;
pub mod error;
pub mod hash;
pub mod schedule;
pub mod storage;

pub use chunking::{Chunk, Chunker, DEFAULT_CHUNK_SIZE};
pub use crypto::{
    decrypt, encrypt, encryption_key_from_secret_data, Ciphertext, EncryptionKey,
    ENCRYPTION_KEY_SECRET_KEYS,
};
pub use error::{CoreError, CoreResult};
pub use hash::{hash_bytes, ContentId};
pub use schedule::{
    backup_run_name, last_tick_at_or_before, next_run_after, normalize_cron_expression,
    parse_schedule, schedule_is_due, validate_schedule,
};
pub use storage::{
    credentials_from_secret_data, expand_local_path, normalize_s3_endpoint, LocalBackend,
    ObjectStore, PutOptions, S3Backend, S3Config, S3Credentials, StoredObject,
};

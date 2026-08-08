//! CRD types for `proteus.io/v1alpha1`.

#![deny(clippy::unwrap_used)]
#![deny(clippy::expect_used)]
#![deny(clippy::panic)]

mod backup;
mod backup_policy;
mod data_plane;
mod repository;
mod restore;

pub use backup::{
    BackupPhase, ProteusBackup, ProteusBackupSpec, ProteusBackupStatus, RetentionPolicy,
};
pub use backup_policy::{
    BackupPolicyPhase, ProteusBackupPolicy, ProteusBackupPolicySpec, ProteusBackupPolicyStatus,
};
pub use data_plane::DataPlane;
pub use repository::{
    LocalBackendSpec, ProteusRepository, ProteusRepositorySpec, ProteusRepositoryStatus,
    RepositoryBackend, RepositoryPhase, S3BackendSpec,
};
pub use restore::{ProteusRestore, ProteusRestoreSpec, ProteusRestoreStatus, RestorePhase};

pub const GROUP: &str = "proteus.io";
pub const VERSION: &str = "v1alpha1";

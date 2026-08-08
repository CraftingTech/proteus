//! Backup/restore data-plane selection (ADR 0001): `agent` when a Ready node-agent
//! covers the PVC's node and the repository is reachable from the agent; else `exec`.

mod select;

pub use select::{select_plane, PlaneChoice, RepositoryKind};

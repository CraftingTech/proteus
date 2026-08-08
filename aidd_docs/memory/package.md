# Package

What this project ships as a reusable package: its public surface and release policy.

## Public API

- Workspace crates intended as libraries: `proteus-core`, `proteus-crd`, `proteus-api` (each exposes `lib.rs`)
- `proteus-controller` is the binary product, not a library consumers import
- Stable-for-consumers surface today: CRD types in `proteus-crd` and CAS traits/types in `proteus-core`; treat `v1alpha1` CRDs as unstable

## Consumers

- In-workspace path deps only (`Cargo.toml` workspace members)
- Not published to crates.io yet; consume via git/path until a release process exists
- Runtime: Rust MSRV `1.78` (workspace `rust-version`)

## Versioning

- Product version = **git tag** `v*` (tag-first release; no bump commits/PRs)
- Workspace `Cargo.toml` stays at stable `0.0.0-dev`; CI injects the tag into the image build via `PROTEUS_VERSION` so `CARGO_PKG_VERSION` matches the release
- Overlay `deploy/overlays/default` pins GHCR `:main`; release install overrides the Deployment image to the semver tag
- Semver: CRD/`v1alpha1` breaking changes allowed without major bump until GA; document CR migrations when they happen
- Kubernetes API compatibility keyed off `k8s-openapi` feature (`v1_30` today) rather than a separate peer dep for callers

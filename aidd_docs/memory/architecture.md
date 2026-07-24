# Architecture

The macro technical shape: the stack, how the pieces fit, and the decisions behind them. Point to the code, do not restate it.

## Stack

- Rust (edition 2021), Tokio — backup I/O and operator runtime
- Kubernetes operator via `kube` / `kube-derive` / `k8s-openapi`
- Axum + `rust-embed` for HTTP API and embedded UI assets
- Dioxus WASM UI in `crates/proteus-ui` (100% Rust source; wasm-bindgen glue only)
- BLAKE3 + AES-256-GCM + fixed-size chunking in `proteus-core`
- Ship path: Docker image + Kustomize (`deploy/`)

## How it fits together

```mermaid
flowchart LR
  UI[proteus-ui Dioxus] --> API[proteus-api]
  API --> CTRL[proteus-controller]
  CR[CRDs proteus-crd] --> CTRL
  CTRL --> CORE[proteus-core CAS]
  CTRL --> K8S[Single-cluster K8s API / PVCs]
  CORE --> LOCAL[Local FS backend]
  CORE -.-> S3[S3 backend]
  KUST[deploy/kustomize] --> CTRL
```

## Key decisions

- One binary embeds API + UI so day-2 ops share process fate and cluster credentials with the operator
- UI is Dioxus WASM — no Node frontend in the repo
- Product UX inspired by Kopia; runtime is Kube-native CRs + controller
- Users install via container image and/or `kubectl apply -k`
- MVP is single-cluster and PVC-centric
- Libraries use `thiserror`; only the binary edge uses `anyhow`
- CRD API group `proteus.io`, version `v1alpha1` until GA

## Gotchas

- S3 backend returns `CoreError::S3NotImplemented` until wired
- Build UI with `just build-ui` before embedding real assets
- Host `cargo test/clippy --workspace` should `--exclude proteus-ui` (WASM target); prefer `just check`
- `PROTEUS_API_ADDR` defaults to `0.0.0.0:8080`
- Day-to-day commands: root `Justfile` (`just run`, `just deploy`, `just pf`)

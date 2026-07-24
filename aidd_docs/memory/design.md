# Design

UI conventions for the embedded Proteus control plane.

## Surface

- Embedded SPA served by the controller (same origin as `/api`)
- Ops UX inspired by Kopia: repositories, backups, inspect status
- MVP pages: Cluster, Repositories, Backups, PVCs

## Stack

- **100% Rust UI**: Dioxus (WASM) in `crates/proteus-ui`
- Built with `just build-ui` (`dx` + stage into `crates/proteus-ui/dist`)
- Assets embedded via `rust-embed` from `crates/proteus-ui/dist`
- Browser still loads tiny wasm-bindgen glue (generated, not hand-written JS app code)

## Conventions

- API under `/api/v1/…`; UI owns client routes with SPA fallback
- Dev: `just ui` or `just run` (needs kubeconfig)
- Keep the shell utilitarian; avoid decorative dashboard chrome
- Never reintroduce a Node/React/Vite frontend
- Local workflows live in the root `Justfile`

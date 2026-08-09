# Design

UI conventions for the embedded Proteus control plane.

## Surface

- Embedded SPA served by the controller (same origin as `/api`)
- Ops UX inspired by Kopia: repositories, backups, inspect status
- MVP pages: Cluster, Repositories, Backups, Inventory

## Brand

- **Mark:** filled heptagon (Kubernetes-inspired) containing the Proteus shield — `crates/proteus-ui/assets/brand/mark.svg` (`currentColor` heptagon, white shield)
- **Wordmark:** letterforms from the name kit (heptagon **O**) — `crates/proteus-ui/assets/brand/wordmark.svg` (`currentColor`)
- **Favicon:** `crates/proteus-ui/assets/favicon.svg` (mark, kube blue `#3069de`)
- **Nav chrome:** mark + wordmark SVG (not letter-`P`, not Plex for the product name)
- **Accent (locked):** `#3069de` (`--accent`) — K8s-inspired blue; `--accent-2` for secondary emphasis; `--accent-glow` for ambient background
- **Color scheme:** `dark` (default) and `light`, toggled in the nav footer; persisted as `localStorage` `proteus-color-scheme`; first visit follows `prefers-color-scheme` when unset
- Surfaces use tokens (`--panel`, `--hover`, `--surface-*`, …) so both schemes stay coherent
- Source artwork lived under operator `tmp/proteus`; composed heptagon mark is repo-owned — see `assets/brand/README.md`

Page UX polish is a separate pass.

## Stack

- **100% Rust UI**: Dioxus (WASM) in `crates/proteus-ui`
- Built with `just build-ui` (`dx` + stage into `crates/proteus-ui/dist`)
- Assets embedded via `rust-embed` from `crates/proteus-ui/dist`
- Browser still loads tiny wasm-bindgen glue (generated, not hand-written JS app code)

## Conventions

- API under `/api/v1/…`; UI owns client routes with SPA fallback
- Dev: `just ui` or `just run` (needs kubeconfig)
- Ops-first shell: brand is a first-class signal; avoid decorative dashboard chrome
- Prefer CSS tokens (`--accent`, surfaces) over one-off colors
- Never reintroduce a Node/React/Vite frontend
- Local workflows live in the root `Justfile`

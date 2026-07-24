# AI Operating Guidelines

How this team drives AI coding assistants on this project.

## House rules

- Prefer typed errors (`thiserror` in libs, `anyhow` only at binary edges); never `unwrap`/`expect`/`panic!` in production paths
- Do not invent Kubernetes API behavior; confirm against `kube` / CRD types in `proteus-crd`
- Commits stay atomic and intention-revealing; never commit or push unless the user asks
- Write self-explanatory code (Clean Code / Fowler). Comments are rare and only capture non-obvious *why*. Refuse comment-heavy patches
- **100% Rust** including the UI (Dioxus WASM). No Node/React/Vite frontend

## Validation depth

- Quick check after a small change: `cargo check -p <crate>`
- Before merge: `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`

## When the AI drifts

- Reset the session objective in one sentence, re-read `aidd_docs/memory/project-brief.md`, `architecture.md`, and `coding-assertions.md`, then continue

For the general AIDD playbook: <https://github.com/ai-driven-dev/framework>.

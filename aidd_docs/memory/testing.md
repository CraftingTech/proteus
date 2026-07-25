# Testing

How the project is tested: the layers, the tools, and the conventions. Where tests live and how to run them.

## Strategy

- Unit tests live next to the code (`#[cfg(test)]` modules) for CAS primitives and local storage
- No integration / e2e harness against a live cluster yet

## Tools

- Cargo built-in test runner (`cargo test`)
- `tempfile` for local-backend filesystem tests in `proteus-core`

## Conventions

- Prefer testing pure CAS / crypto / chunking without Kubernetes
- Controller and API paths are bootstrap-thin; expand tests when reconcile does real work
- Production code must stay free of `unwrap`/`expect`; tests may use them under clippy test allowances

## Run

- All: `cargo test --workspace`
- One crate: `cargo test -p proteus-core`
- Pre-merge gate: `just check` (fmt + clippy + tests + UI build)

## CI

- GitHub Actions: `.github/workflows/check.yml` runs `just check` on pull requests and pushes to `main`
- Toolchain pinned to Rust `1.91.1` + `dioxus-cli` `0.7.9` (same as `deploy/Dockerfile` / CONTRIBUTING)

# Contributing to Proteus

Thanks for your interest. Proteus is early-stage; small, focused changes are welcome.

## Ground rules

- Discuss large design changes (CRD shape, crate split, storage format) in an issue before a big PR
- Keep production code free of `unwrap` / `expect` / `panic!`
- Do not commit secrets, kubeconfigs, or real credentials

## Dev setup

- Rust stable (MSRV: see `rust-version` in the workspace `Cargo.toml`)
- [`just`](https://github.com/casey/just)
- `dioxus-cli` 0.7.9 for the UI
- A Kubernetes cluster + `kubectl` for operator smoke tests

```bash
just check    # fmt + clippy + tests + UI build
just run      # controller against your kubeconfig
just smoke    # curl healthz + /api/v1/cluster
```

See `just --list` for the full recipe set.

## Pull requests

1. Fork and branch from `main` (`feat/…`, `fix/…`, or `chore/…`)
2. Prefer Conventional Commits: `feat(core): …`, `fix(crd): …`, etc.
3. Include tests when changing CAS / crypto / reconcile behavior
4. Update docs or `aidd_docs/memory/` when you change purpose, architecture, or public contracts
5. CI runs `just check` + `kubectl kustomize deploy/overlays/default` on every PR; keep that green before merge
6. Images publish from `main` / tags via `.github/workflows/image.yml` (no Helm — Kustomize only)

## Project docs

| Doc | Audience |
| --- | -------- |
| [`README.md`](README.md) | Humans landing on the repo |
| [`Justfile`](Justfile) | Local commands |
| [`aidd_docs/memory/project-brief.md`](aidd_docs/memory/project-brief.md) | Purpose / domain language |
| [`aidd_docs/memory/architecture.md`](aidd_docs/memory/architecture.md) | Stack and decisions |
| [`aidd_docs/CONTRIBUTING.md`](aidd_docs/CONTRIBUTING.md) | How to change AI project memory |

## License

By contributing, you agree that your contributions are licensed under the Apache License 2.0 (`LICENSE`).

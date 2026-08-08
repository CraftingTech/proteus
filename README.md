# Proteus

Kube-native backup and disaster recovery with an embedded operator UI — **100% Rust**.

Proteus runs as a Kubernetes controller, owns its content-addressable storage (chunking, BLAKE3, AES-256-GCM, compression), and exposes a **Kopia-inspired UI** (Dioxus WASM) to configure backup destinations and drive backup/restore.

> **Status:** early MVP / pre-release (CRDs `v1alpha1`; product version = git tag, e.g. `v0.0.1-alpha.2`). Usable local+S3 PVC backup/restore + scheduled policies; not a finished product.

## Why it exists

Canonical brief: [`aidd_docs/memory/project-brief.md`](aidd_docs/memory/project-brief.md) · architecture: [`aidd_docs/memory/architecture.md`](aidd_docs/memory/architecture.md)

## MVP scope

- Embedded UI (required) — Dioxus
- List / back up PVCs
- One Kubernetes cluster
- Repository setup from the UI (S3-compatible, local path/URL-style targets)

## Workspace

| Path | Role |
| ---- | ---- |
| `crates/proteus-core` | CAS engine |
| `crates/proteus-crd` | CRD types |
| `crates/proteus-api` | Axum API + embedded UI assets |
| `crates/proteus-controller` | Operator binary |
| `crates/proteus-ui` | Dioxus WASM SPA |
| `deploy/` | Dockerfile + Kustomize install |
| `Justfile` | All local workflows |

## Prerequisites

- Rust (MSRV in workspace `Cargo.toml`)
- [`just`](https://github.com/casey/just)
- [`dioxus-cli`](https://dioxuslabs.com/) `0.7.9` (`cargo install dioxus-cli --version 0.7.9 --locked`)
- `kubectl` + a kubeconfig for `just run` / `just deploy`

```bash
just              # list recipes
just check        # pre-commit gates
just run          # controller + UI (~/.kube/config)
just run kubeconfig=/path/to/config
just cleanup      # remove Proteus CRDs from the cluster
```

## Install on a cluster

Release path (tag-first): apply the Kustomize overlay at a git tag, then pin the GHCR
image to the same semver (no leading `v`):

```bash
TAG=0.0.1-alpha.2
kubectl apply -k "https://github.com/CraftingTech/proteus.git//deploy/overlays/default?ref=v${TAG}"
kubectl -n proteus-system set image deploy/proteus-controller \
  controller=ghcr.io/craftingtech/proteus-controller:${TAG}
just pf                           # port-forward UI → :8080
```

Tip-of-tree / local image:

```bash
just image                        # docker build → proteus-controller:local
just deploy                       # kubectl apply -k deploy/overlays/default (pins :main)
```

Details and release process: [`deploy/README.md`](deploy/README.md). After changing CRD types: `just crds`.

## Contributing

See [`CONTRIBUTING.md`](CONTRIBUTING.md). AI-assisted workflow notes live under [`aidd_docs/`](aidd_docs/).

## License

Apache License 2.0 — see [`LICENSE`](LICENSE).

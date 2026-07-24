# Deploy Proteus

All of this is also available via the root [`Justfile`](../Justfile) (`just deploy`, `just image`, `just pf`, `just crds`).

## Container image

```bash
just image
# or:
docker build -f deploy/Dockerfile -t proteus-controller:local .
```

The image builds the Dioxus WASM UI (`dx`) then the Rust controller, and embeds UI assets in the binary.

## Install with Kustomize

```bash
just deploy
# or:
kubectl apply -k deploy/kustomize/overlays/default
```

Port-forward the UI:

```bash
just pf
open http://127.0.0.1:8080
```

## CRDs

Types live in `crates/proteus-crd`. After changing them:

```bash
just crds
```

This overwrites YAML under `deploy/kustomize/crds/` from `kube::CustomResourceExt`.

## Sample resources

```bash
just samples
```

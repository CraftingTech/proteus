# Deployment

How Proteus is built and shipped to a Kubernetes cluster.

## Tooling

- Container image: multi-stage `deploy/Dockerfile` (Dioxus WASM UI → Rust release → distroless runtime)
- Cluster install: Kustomize under `deploy/` (`base` + `overlays/default`) — **no Helm**
- CRDs live in `deploy/crds/` and are applied with the base
- Image registry: `ghcr.io/craftingtech/proteus-controller` (CI multi-arch `amd64`/`arm64`)

## Topology

```mermaid
flowchart TD
  IMG[Container image] --> DEP[Deployment proteus-controller]
  IMG --> DS[DaemonSet proteus-node-agent]
  DEP --> SA[ServiceAccount + ClusterRole]
  DEP --> SVC[Service proteus :80]
  SVC --> UI[Embedded UI + API :8080]
  DEP --> CRD[CRDs proteus.io]
  DEP --> PVC[Orchestrate PVC backup/restore]
  DS --> MOVER[Mover Pods mount PVCs]
  MOVER --> CAS[Remote CAS / S3]
```

Controller Deployment and **node-agent DaemonSet** ship in `deploy/base`. Release installs `kubectl set image` both `deploy/proteus-controller` and `ds/proteus-node-agent`. Agent discovers mover image from its own Pod spec. CSI VolumeSnapshots remain later (#66).

## Conventions

- Image name default: `ghcr.io/craftingtech/proteus-controller:<tag>` (controller, agent, and mover modes share this image)
- Tip install: `kubectl apply -k deploy/overlays/default` (pins `:main`)
- Release install: apply overlay at git tag `vX.Y.Z`, then `kubectl set image` Deployment **and** DaemonSet to `:X.Y.Z` (tag-first; no bump commits)
- Image build ARG `PROTEUS_VERSION` bakes release semver into `CARGO_PKG_VERSION`
- Local repo data path in-cluster: `/var/lib/proteus` (emptyDir in base; replace with PVC for persistence) — forces **exec** data plane
- GitOps: consumers may Argo-source this repo (`path: deploy/overlays/default`) or vendor `deploy/` into their own GitOps repo; Ingress is a consumer overlay
- Large-PVC throughput: S3-compatible repo + Ready agents on PVC nodes → `dataPlane=agent`; otherwise exec fallback

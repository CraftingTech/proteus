# Infrastructure

How the runtime is provisioned: infrastructure as code and topology.

## Tooling

- Kustomize manifests under `deploy/` (`base` + `overlays/default`) — no Helm
- Container build: `deploy/Dockerfile`
- Example CRs: `deploy/examples/sample-resources.yaml`

## Topology

```mermaid
flowchart TD
  API[K8s API] --> CTRL[proteus-controller Pod]
  CTRL --> CR[ProteusRepository / Backup / Restore]
  CTRL --> VOL[/var/lib/proteus or S3]
  CTRL --> HTTP[UI + API Service]
```

## Conventions

- Prefer `kubectl apply -k deploy/overlays/default` as the documented install path
- Overlay owns image registry/tag; base owns RBAC, probes, and CRDs
- Sample CRs are optional post-install examples, not part of the base package

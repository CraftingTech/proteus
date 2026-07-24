# Infrastructure

How the runtime is provisioned: infrastructure as code and topology.

## Tooling

- Kustomize manifests under `deploy/kustomize/` (base + `overlays/default`)
- Container build: `deploy/Dockerfile`
- Example CRs: `deploy/examples/sample-resources.yaml`
- No Helm / Terraform in-repo yet

## Topology

```mermaid
flowchart TD
  API[K8s API] --> CTRL[proteus-controller Pod]
  CTRL --> CR[ProteusRepository / Backup / Restore]
  CTRL --> VOL[/var/lib/proteus or S3]
  CTRL --> HTTP[UI + API Service]
```

## Conventions

- Prefer `kubectl apply -k deploy/kustomize/overlays/default` as the documented install path
- Overlay owns image registry/tag; base owns RBAC, probes, and CRDs
- Sample CRs are optional post-install examples, not part of the base package

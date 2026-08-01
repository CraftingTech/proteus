# ADR 0001 — Production data plane (node-agent + CSI)

- **Status:** Accepted (product decision 2026-08-01)
- **Date:** 2026-08-01
- **Epic:** [#66](https://github.com/CraftingTech/proteus/issues/66)
- **Related:** [#24](https://github.com/CraftingTech/proteus/issues/24), [#28](https://github.com/CraftingTech/proteus/issues/28), [#36](https://github.com/CraftingTech/proteus/issues/36)

## Context

MVP Proteus backs up PVC data by starting a short-lived mount Pod and streaming `tar` over **kube-exec** into the controller process (often through the API server). Streaming removed the full-archive RAM buffer, but large-volume throughput remains unacceptable (#24).

An earlier M5 design used a **per-backup Job + custom image**. That was vetoed for ops friction (homelab/Pi) and to preserve `just run`. Compression (#36) can mitigate bytes on the wire / in the repo but does **not** replace a real data plane.

Velero/Kopia-class tools move volume bytes **on the node** (DaemonSet node-agent) and optionally via **CSI VolumeSnapshots**. Proteus needs an equivalent production shape to become a credible product — not only a clever MVP.

## Decision

Ship **both** of the following as the production data plane. Keep today’s exec path as an explicit **fallback / early-dev** mode.

### 1. DaemonSet `proteus-node-agent`

- One agent Pod per node (Kustomize DaemonSet).
- Controller **orchestrates** `ProteusBackup` / `ProteusRestore` CRs; the agent performs **bulk data movement** into/out of `proteus-core` CAS (chunk → hash → optional compress → encrypt → put).
- Production backups of PVC filesystem data **must not** stream bulk payloads through the API server into the controller Deployment.

### 2. CSI VolumeSnapshot path (#28)

- When VolumeSnapshot CRDs + a supporting CSI driver/storage class exist: create snapshot → materialize readable volume (PVC from snapshot) → data-move into CAS via the agent (or a short-lived mover Pod using the same binary).
- When CSI is unavailable: degrade to **agent FS** path, then to **exec** if no agent is present.
- Snapshotting improves crash-consistency vs live tar of a dirty filesystem; it does not replace CAS encryption/dedup.

### 3. Packaging: one image, two modes

- Prefer **one** multi-arch image (`ghcr.io/craftingtech/proteus-controller`) with a process mode:
  - controller / API / UI (Deployment) — default
  - node-agent (DaemonSet) — e.g. `proteus-controller agent` or `PROTEUS_MODE=agent`
- Avoid a second unrelated image repository unless forced by size/security splitting later.
- M5 veto remains for **Job-per-backup spaghetti**; DaemonSet + optional short-lived mover Pods owned by the agent/controller are in scope.

### 4. Fallback matrix

| Agent DaemonSet | CSI snapshots | Production path | Notes |
| --- | --- | --- | --- |
| Present | Available + allowed | **csi** (preferred when policy says so) | Status `dataPlane: csi` |
| Present | Missing / failed | **agent** FS | Status `dataPlane: agent` |
| Absent | * | **exec** (mount Pod + kube-exec) | Status `dataPlane: exec`; document as unsupported for large PVCs |
| Local `just run` | n/a | **exec** by default | See Dev story |

Selection order (default policy): `csi` → `agent` → `exec`, overridable later via Repository/Backup knobs if needed.

### 5. Dev story (`just run` / kind)

- **Local controller** (`just run` against a kubeconfig): default `PROTEUS_DATA_PLANE=exec` (or auto-detect: no agent endpoints → exec). Contributors keep a one-process loop for API/UI/CRD work.
- **Kind / real cluster**: `just deploy` (or overlay) installs controller **and** node-agent DaemonSet. Document a minimal kind recipe; accept that “prod-like backup” needs the agent.
- Large-PVC performance claims require cluster-deployed agent (+ CSI when testing that path). Exec remains for smoke tests and contrib UX, not for #24 success criteria.

### 6. Status / observability

- Backup (and restore) status records which plane ran: `exec` | `agent` | `csi`.
- Keep `durationSeconds` / `throughputBytesPerSec` (#64) on all planes for apples-to-apples baselines.

### 7. Volume access (agent FS) — provisional

Industry pattern (Velero FSB): node-agent reaches pod volume data via **node-local** access (commonly hostPath under the kubelet pod directory) with elevated `SecurityContext`.

Proteus adopts that **direction** for live FS backup, with an implementation spike to confirm:

- required privileges / PSS (privileged / hostPath) on target distros (incl. k3s / Pi);
- whether a less-privileged alternative (agent-scheduled mount Pod + **local** handoff that never crosses the apiserver data plane) is viable for v1 of the agent.

CSI path prefers: snapshot → PVC-from-snapshot → mount in mover/agent Pod (standard CSI), avoiding hostPath when possible.

*Provisional* means the privilege details may tighten in a follow-up ADR amendment; the decision to use a DaemonSet node-local data plane is not provisional.

### 8. Compression

- CAS per-chunk compression remains [#36](https://github.com/CraftingTech/proteus/issues/36) — orthogonal.
- Wire-side compactation on the exec fallback is optional and secondary to this ADR.

## Consequences

### Positive

- Aligns Proteus with how operators expect backup products to move data.
- Unblocks realistic large-PVC performance (#24) without inventing a proprietary Job image per run.
- CSI path offers better consistency where storage supports it (#28).
- Single image keeps install story Kustomize-simple.

### Negative / costs

- Install surface grows: DaemonSet, broader RBAC, possible privileged/hostPath requirements.
- `just run` alone is no longer sufficient to validate production throughput.
- CSI dependency is cluster-specific; must degrade cleanly.
- Larger design/implementation surface before “feels like a product”.

### Out of scope (this ADR)

- Multi-cluster / cross-cluster
- Replacing CAS format or CRD group
- Mandatory compression
- Helm

## Implementation order

1. Land this ADR in-repo + link from `aidd_docs/memory/architecture.md` (milestone A / #66).
2. Node-agent MVP: DaemonSet + agent FS path + status + exec fallback (milestone B).
3. CSI snapshot path via agent/mover (#28) (milestone C).
4. UI/docs/load tests (milestone D).

## References

- Epic #66
- Velero File System Backup / node-agent (Kopia uploader) — node-local data movement
- Proteus M5 note: Job/image rejected; streaming exec kept for MVP

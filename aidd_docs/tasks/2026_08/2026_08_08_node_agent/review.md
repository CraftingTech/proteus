# Review: node_agent

- **Verdict**: changes-requested
- **Diff**: `main...feat/66-node-agent`
- **Axes run**: code, functional, relevancy
- **Date**: 2026_08_08
- **Findings**: 1 critical, 3 warning, 2 minor

## Phases

### Phase 1 — Binary modes + DaemonSet skeleton

- [x] `proteus-controller agent` starts without launching the HTTP API — `crates/proteus-controller/src/main.rs:21`
- [x] `kubectl kustomize deploy/overlays/default` includes DaemonSet `proteus-node-agent` — `deploy/base/daemonset.yaml:4` / kustomize render verified

### Phase 2 — dataPlane status + plane selection

- [x] CRD YAML contains dataPlane on Backup and Restore status — `deploy/crds/proteusbackups.yaml:91`, `deploy/crds/proteusrestores.yaml:66`
- [x] Unit tests: Local→exec, no agent→exec, Ready agent+S3→agent — `crates/proteus-controller/src/data_plane/select.rs:277` (4 tests pass)

### Phase 3 — Agent backup + mover ingest

- [x] Mover can be invoked with args without starting API — `crates/proteus-controller/src/main.rs:23`, `crates/proteus-controller/src/agent/mover.rs:29`
- [ ] On cluster with agent+S3: Backup reaches Succeeded with dataPlane=agent — gap: no cluster evidence in diff (static review only); tag `not-applicable`

### Phase 4 — Agent restore + mover extract

- [ ] Restore mover writes snapshot data to `/data` without kube-exec — gap: mounts use `/volumes/<pvc>` not `/data`; no kube-exec ✅ path exists at `mover.rs:246` but criterion wording unmet on mount path; tag `fix`
- [ ] Cluster: Restore Succeeded with dataPlane=agent after agent Backup — gap: no cluster evidence; tag `not-applicable`

### Phase 5 — Docs + API/UI + unit tests

- [x] deploy README documents DaemonSet + set-image still works — `deploy/README.md` data-plane table + DS `set image`
- [x] UI/API list/detail include dataPlane; plane-selection tests pass — `crates/proteus-api/src/resources/backups.rs`, `restores.rs`, UI pills; select tests pass

## Findings

| Sev | Kind | Phase | Location | Issue | Fix |
| --- | ---- | ----- | -------- | ----- | --- |
| 🔴 | code | 4 | `crates/proteus-controller/src/agent/mover.rs:246` | Restore agent path buffers full volume via `materialize_volume` then `write_all` — OOM risk on large PVCs; undermines “same speed class / no full-archive” goal vs backup streaming | Stream CAS chunks into `tar -xf` (or pipe) without holding the whole archive |
| 🟡 | functional | 4 | `crates/proteus-controller/src/agent/mover.rs:232` | Phase 4 criterion expects write under `/data`; implementation mounts `/volumes/<pvc>` | Align criterion/docs to `/volumes/<pvc>` or remount at `/data` for single-PVC movers |
| 🟡 | conform | 3 | `deploy/base/clusterrole-agent.yaml:16` | Agent ClusterRole can `create` ClusterRoleBindings cluster-wide — broad privilege for a DaemonSet | Prefer RoleBinding to a namespaced Role, or a single pre-created CRB with dynamic subjects via a narrower controller |
| 🟡 | rot | 3 | `crates/proteus-controller/src/agent/work.rs:343` + `pvc_reader.rs` | Mover Pod wait/create/label patterns duplicate mount-Pod orchestration in backup/restore exec path | Extract shared short-lived Pod helper (create/wait/delete by labels) |
| 🟢 | code | 1 | `crates/proteus-controller/src/agent/mod.rs:46` | `std::env::set_var` after process start is process-global and surprising for tests/future threads | Pass image via `Arc`/struct into `work` instead of mutating env |
| 🟢 | code | 3 | `crates/proteus-controller/src/agent/work.rs:35` | Cluster-wide list+poll every 5s for all Backup/Restore — fine for MVP, noisy at scale | Watch with field selectors / limit concurrency when volume grows |

## Verification

| Metric        | Value                                             |
| ------------- | ------------------------------------------------- |
| Verified      | 70% (7/10)                                        |
| Files checked | `main.rs`, `agent/mod.rs`, `agent/mover.rs`, `agent/work.rs`, `data_plane/select.rs`, `deploy/base/daemonset.yaml`, `deploy/crds/*`, `deploy/README.md`, `proteus-api/resources/{backups,restores}.rs`, `proteus-ui/{api,pages}/*`, `clusterrole-agent.yaml`, `Dockerfile` |
| Unchecked     | Phase3 cluster Backup Succeeded — not-applicable; Phase4 `/data` mount wording — fix; Phase4 cluster Restore Succeeded — not-applicable |
| Unplanned     | Dynamic SA+ClusterRoleBinding provisioning in workload namespaces; Busybox `tar` copied into distroless image |

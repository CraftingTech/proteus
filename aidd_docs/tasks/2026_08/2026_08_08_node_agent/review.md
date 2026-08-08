# Review: node_agent

- **Verdict**: approve
- **Diff**: `main...feat/66-node-agent`
- **Axes run**: code, functional, relevancy
- **Date**: 2026_08_08
- **Findings**: 0 critical, 1 warning, 1 minor

## Phases

### Phase 1 — Binary modes + DaemonSet skeleton

- [x] `proteus-controller agent` starts without launching the HTTP API — `crates/proteus-controller/src/main.rs:21`
- [x] `kubectl kustomize deploy/overlays/default` includes DaemonSet `proteus-node-agent` — `deploy/base/daemonset.yaml:4`

### Phase 2 — dataPlane status + plane selection

- [x] CRD YAML contains dataPlane on Backup and Restore status — `deploy/crds/proteusbackups.yaml:91`, `deploy/crds/proteusrestores.yaml:66`
- [x] Unit tests: Local→exec, no agent→exec, Ready agent+S3→agent — `crates/proteus-controller/src/data_plane/select.rs:277`

### Phase 3 — Agent backup + mover ingest

- [x] Mover can be invoked with args without starting API — `crates/proteus-controller/src/main.rs:23`, `crates/proteus-controller/src/agent/mover.rs:29`
- [ ] On cluster with agent+S3: Backup reaches Succeeded with dataPlane=agent — gap: static review only; tag `not-applicable`

### Phase 4 — Agent restore + mover extract

- [x] Restore mover writes snapshot data to `/volumes/<pvc>` without kube-exec or full-volume buffer — `mover.rs` + `materialize_volume_to_writer`
- [ ] Cluster: Restore Succeeded with dataPlane=agent after agent Backup — gap: static review only; tag `not-applicable`

### Phase 5 — Docs + API/UI + unit tests

- [x] deploy README documents DaemonSet + set-image still works — `deploy/README.md`
- [x] UI/API list/detail include dataPlane; plane-selection tests pass — API/UI + select tests

## Findings

| Sev | Kind | Phase | Location | Issue | Fix |
| --- | ---- | ----- | -------- | ----- | --- |
| 🟡 | rot | 3 | `agent/work.rs` + `backup/pvc_reader.rs` | Short-lived Pod create/wait/delete still duplicated vs exec mount path | Extract shared helper in a follow-up (non-blocking for MVP) |
| 🟢 | code | 3 | `agent/work.rs:34` | Cluster-wide list+poll every 5s — OK for MVP | Watch / concurrency limits later |

## Verification

| Metric        | Value                                             |
| ------------- | ------------------------------------------------- |
| Verified      | 80% (8/10)                                        |
| Files checked | `pipeline.rs`, `mover.rs`, `work.rs`, `identity.rs`, `clusterrole-*.yaml`, `backup.rs`, `restore.rs`, `phase-4.md`, `agent/mod.rs` |
| Unchecked     | Phase3/4 cluster E2E — not-applicable             |
| Unplanned     | none                                              |

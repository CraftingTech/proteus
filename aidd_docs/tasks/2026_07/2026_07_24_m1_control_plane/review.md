# Review: M1 Live control plane + cluster inventory

- **Verdict**: approve
- **Diff**: `main...feat/m1-control-plane`
- **Axes run**: code, functional, relevancy
- **Date**: 2026_07_24
- **Findings**: 0 critical, 0 warning, 1 minor

## Phases

### Phase 1 — API: live state, CR lists, inventory, readyz

- [x] After applying sample CRs, `/api/v1/cluster` counts are > 0 — `crates/proteus-api/src/state.rs:91-114` (`refresh_counts`); controllers call it on reconcile (`repository.rs`, `backup.rs`, `restore.rs`); startup refresh in `main.rs:33-35`
- [x] List endpoints return those CRs with name/namespace — `crates/proteus-api/src/routes.rs:23-25`, `resources.rs:50-113`
- [x] Inventory returns only allowed kinds; Secret payloads absent — `inventory.rs:14-21`, `54-71`, `253-266` (`list_metadata` / `PartialObjectMeta`); unit test `inventory.rs:310-327`
- [x] Without kube/CRDs, `/readyz` is not 200; healthy start → 200 — `routes.rs:42-54`, `state.rs:63-88` + `124-137`; ClusterRole grants CRD `get`/`list` (`deploy/kustomize/base/clusterrole.yaml:19-20`); 401/403 distinguished from unreachable (`state.rs:76-81`, `117-122`)

### Phase 2 — UI: Cluster live stats + CR list pages

- [x] UI compiles; fetches hit relative API paths — `crates/proteus-ui/src/api.rs:78-91`; `just build-ui` succeeded
- [x] Embedded UI on `:8080` shows live counts matching API — `pages/cluster.rs:6-27` loads `get_cluster()`; relative `/api/v1/cluster`
- [x] Sample CRs appear as rows after apply — `pages/repositories.rs`, `pages/backups.rs` bind list APIs (name/namespace columns)

### Phase 3 — UI: cluster inventory with filters

- [x] Operator filters by namespace/kind/name and sees matching objects — `pages/inventory.rs:16-81` + API `list_inventory`; ClusterRole covers pods/services/deployments/secrets/configmaps/PVCs (`clusterrole.yaml:21-32`)
- [x] Secret rows never display secret data — `pages/inventory.rs:41`, `114-119` render `extra` only; API `secret_from_partial_meta` omits values (`inventory.rs:259-266`)
- [x] PVC objects visible via Kind=PVC — API alias `inventory.rs:73-75`; UI option `PersistentVolumeClaim` (`inventory.rs:9`); ClusterRole allows PVC list

## Findings

| Sev | Kind | Phase | Location | Issue | Fix |
| --- | ---- | ----- | -------- | ----- | --- |
| 🟢 | fit | 1 | `crates/proteus-api/src/inventory.rs:259-265` | Phase-1 task / wireframe asked Secret `type` + keys-count in `extra`; `PartialObjectMeta` path correctly omits `.data` but also drops type/keys (`extra: None`) | Optional follow-up: surface non-sensitive type from annotations/labels or document the metadata-only tradeoff; do not reintroduce full Secret list |

## Verification

| Metric        | Value                                             |
| ------------- | ------------------------------------------------- |
| Verified      | 100% (10/10)                                      |
| Files checked | `crates/proteus-api/src/{state,routes,resources,inventory,error,lib}.rs`, `crates/proteus-controller/src/{main.rs,controllers/*}`, `crates/proteus-ui/src/{api,main,shell,pages/*}`, `deploy/kustomize/base/clusterrole.yaml`, `aidd_docs/memory/api.md`, plan/phase-*.md |
| Unchecked     | live kube exercise (sample CRs / real `/readyz` / inventory against cluster) — not-applicable (no usable cluster here; static + `cargo test -p proteus-api` 5 ok, `clippy` clean, `fmt-check` ok, `just build-ui` ok) |
| Unplanned     | none (docs/memory + UI styles + `pvcs.rs`→inventory swap trace to plan) |

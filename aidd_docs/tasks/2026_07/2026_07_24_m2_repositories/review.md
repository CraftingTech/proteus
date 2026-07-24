# Review: M2 Repositories (local + S3)

- **Verdict**: approve
- **Diff**: `main...feat/m2-repositories`
- **Axes run**: code, functional, relevancy
- **Date**: 2026_07_24
- **Findings**: 0 critical, 0 warning, 3 minor

## Phases

### Phase 1 — API CRUD for ProteusRepository

- [x] Invalid local/s3 body → 4xx with message naming the field — `crates/proteus-api/src/resources.rs:122-127`, `136-173`, tests `330-381`
- [x] POST local repo → CR exists; GET returns it; DELETE removes it — `create_repository`/`get_repository`/`delete_repository` `209-283`; routes `crates/proteus-api/src/routes.rs` POST/GET/PATCH/DELETE
- [x] Existing GET list still works — `list_repositories` `204-208` + route GET `/api/v1/repositories`

### Phase 2 — Controller: validate + Ready/Failed status

- [x] Valid local path → Ready — `LocalBackend::probe` + reconcile Ready branch `crates/proteus-controller/src/controllers/repository.rs:36-43`, `76-83`
- [x] Impossible path → Failed with message — probe Err → Failed `44-50`; unit `crates/proteus-core/src/storage/local.rs` `probe_rejects_impossible_path`
- [x] Missing/wrong secret → Failed — `load_s3_credentials` `120-132` → Failed status
- [x] List/UI show phase Ready or Failed — list `message`/`phase` in API item; UI badges `crates/proteus-ui/src/pages/repositories.rs:364-372`

### Phase 3 — UI: create/list local + S3 repositories

- [x] Can create a local repo from UI; row appears with status — form Local + `create_repository` `90-98`, `134-148`, `211+`
- [x] Can create an S3-shaped repo from UI (CR created; Ready depends on probe) — S3 fields `99-120`, form S3 panel
- [x] Delete removes the CR and row — confirm + `delete_repository` `10-15`, `376-399`

### Phase 4 — Real S3-compatible ObjectStore + reconcile probe

- [x] `S3NotImplemented` gone; put then get round-trips — `CoreError::S3`; InMemory `put_get_round_trip` `crates/proteus-core/src/storage/s3.rs`
- [x] Repo with bad endpoint/secret → Failed; good MinIO config can → Ready — probe wiring `85-95`; optional live test `live_minio_round_trip_when_configured`
- [x] README or deploy note mentions MinIO-compatible settings — `deploy/README.md:46-96`

## Findings

| Sev | Kind | Phase | Location | Issue | Fix |
| --- | ---- | ----- | -------- | ----- | --- |
| 🟢 | rot | 4 | `crates/proteus-core/src/storage/s3.rs:42-52`, `deploy/README.md:48-54` | Accepted Secret key-pair list still duplicated (architecture already links to README) | Keep canonical table in `deploy/README.md`; point the rustdoc at that URL only |
| 🟢 | code | 3 | `crates/proteus-ui/src/pages/repositories.rs:17-70` | Page still owns ~15 signals + form + poll + table in one component | Extract form state / row actions when the page grows again |
| 🟢 | conform | 1 | `crates/proteus-crd/src/repository.rs:105-106` | CRD/serde `force_path_style` default remains `false` while API create/PATCH now `unwrap_or(true)`; bare kubectl YAML omitting the field still virtual-hosted | Align CRD/OpenAPI default to `true`, or document loudly that raw YAML must set `forcePathStyle: true` for MinIO |

## Verification

| Metric        | Value                                             |
| ------------- | ------------------------------------------------- |
| Verified      | 100% (13/13)                                      |
| Files checked | `crates/proteus-api/src/{resources,routes,error}.rs`, `crates/proteus-controller/src/controllers/repository.rs`, `crates/proteus-core/src/storage/{s3,local}.rs`, `crates/proteus-core/src/error.rs`, `crates/proteus-crd/src/repository.rs`, `crates/proteus-ui/src/{api,pages/repositories}.rs`, `crates/proteus-ui/assets/styles.css`, `deploy/README.md`, `aidd_docs/memory/{api,architecture,integration}.md`, phase + plan docs, fix commit `d2a27e7` |
| Unchecked     | live kube e2e (API create → reconcile Ready/Failed; UI create/delete in cluster) — not-applicable (no cluster harness; static + unit only); live MinIO (`PROTEUS_S3_ENDPOINT`) — not-applicable (optional; skipped in CI) |
| Unplanned     | none (PVC backup via S3 deferred M3 per plan)     |

Prior warnings resolved in `d2a27e7`: API `force_path_style.unwrap_or(true)` on create/PATCH via `backend_from_request` (+ unit tests); UI 7s poll while phase `None`/`Pending` + Refresh; `S3Credentials` Debug redacts secret (+ unit test). Checker: `just fmt-check` OK; `just clippy` OK; `cargo test -p proteus-api --lib` 12 passed; `cargo test -p proteus-controller` 2 passed; `cargo test -p proteus-core --lib` 16 passed; `just build-ui` OK. Live kube e2e not exercised.

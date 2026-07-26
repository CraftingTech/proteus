# Codebase Audit: crates/ (code-quality)

Santé **fair** — bon DIP sur `ObjectStore` + pipeline, mais dette SOLID/DRY claire (god-modules, duplication open-repo/secrets/fingerprint) et un footgun data-loss sur `delete_snapshot`.

- **Date**: 2026_07_26
- **Scope**: `crates/` (workspace Proteus), pilier `code-quality` (Clean Code + SOLID)
- **Health**: fair
- **Findings**: 1 critical, 11 warning, 5 minor

Health: `good` = no critical findings; `fair` = critical findings exist but are isolated and addressable; `poor` = systemic or widespread critical findings.

## Findings

| Sev | Category     | Location | Issue | Suggested fix | Effort |
| --- | ------------ | -------- | ----- | ------------- | ------ |
| 🔴 | code-quality | `crates/proteus-core/src/backup/pipeline.rs:252` | Correctness / data loss — `delete_snapshot` deletes every referenced chunk with no refcount; shared CAS chunks from other snapshots are destroyed (comment admits MVP gap) | Prefer mark&sweep only (`gc_unreferenced`); remove or gate destructive delete behind explicit unsafe/refcount | L |
| 🟡 | code-quality | `crates/proteus-core/src/backup/pipeline.rs:52` | Dead contract — prod puts use `PutOptions::default()` (`skip_if_exists: false`) while CAS dedup API exists and is tested (`s3.rs:408`); write-path dedup never armed | Use `PutOptions { skip_if_exists: true }` on chunk puts (keep overwrite or create for manifests if needed) | S |
| 🟡 | code-quality | `crates/proteus-api/src/resources.rs:1` | SRP — god module (~1350 lines): DTOs, validation, Secret upsert, CRUD repo/backup/restore, GC, store open | Split into `repositories` / `backups` / `restores` + `secrets` + `repo_store` | L |
| 🟡 | code-quality | `crates/proteus-api/src/resources.rs:839` vs `crates/proteus-controller/src/backup/repo.rs:43` | DRY + OCP — Local/S3 open duplicated (API GC vs controller); new backend = N call sites | Shared `open_repository_store(...)` factory (core-agnostic config + thin K8s loaders) | M |
| 🟡 | code-quality | `crates/proteus-controller/src/backup/repo.rs:95` (+ `controllers/repository.rs:204`, `api/resources.rs:486`/`890`) | DRY — Secret decode logic copied (string vs raw) across API and controller | Shared `k8s_secret::{decode_string_map, decode_raw_map}` | S |
| 🟡 | code-quality | `crates/proteus-controller/src/controllers/backup.rs:192` (+ `restore.rs:125`, `repository.rs:88`) | DRY — `message_fingerprint` / digit collapse triplicated | `controllers/status_fingerprint.rs` | S |
| 🟡 | code-quality | `crates/proteus-core/src/storage/local.rs:96` vs `s3.rs:234` | LSP — Local `skip_if_exists` is check-then-write (TOCTOU); S3 uses `PutMode::Create`; substitutability imperfect | Local: `create_new` / `O_EXCL` (or fail on rename collision) before write | M |
| 🟡 | code-quality | `crates/proteus-api/src/resources.rs:340` vs `:415` | DRY — `upsert_s3_credentials_secret` ≈ `upsert_encryption_secret` (labels, ownerRef, create/409/patch) | Generic `upsert_opaque_secret(name, string_data, owner)` | S |
| 🟡 | code-quality | `crates/proteus-core/src/backup/pipeline.rs:209` vs `:46` | DRY — `create_snapshot_with_progress` reimplements ingest loop instead of calling `ingest_volume_backup_with_progress` | Delegate + progress bridge | S |
| 🟡 | code-quality | `crates/proteus-controller/src/backup/mod.rs:31` (+ `restore/mod.rs`, `repo.rs`) | Error handling — domain paths return `Result<_, String>`; loses typed `thiserror` matching/tests | Dedicated `BackupError` / `RestoreError` + `From` → status message | M |
| 🟡 | code-quality | `crates/proteus-controller/src/backup/pvc_reader.rs:17` vs `restore/pvc_writer.rs:15` | DRY — mount-pod helpers (`MOUNT_IMAGE`, `short_id`, build/wait) duplicated RO vs RW | Shared `mount_pod` module parameterized by purpose | M |
| 🟡 | code-quality | `crates/proteus-ui/src/pages/backups.rs:86` | SRP — page god (~905 lines): list + create backup + create restore + delete + helpers | Split `backup_list` / `backup_form` / `restore_form` | M |
| 🟢 | code-quality | `crates/proteus-controller/src/main.rs:1` | Craft gap — `proteus-core`/`api`/`crd` deny `clippy::unwrap_used`; controller (and UI) do not | Add same `#![deny(clippy::unwrap_used)]` (fix any real prod unwraps) | S |
| 🟢 | code-quality | `crates/proteus-core/src/storage/s3.rs:48` | SRP light — K8s Secret key alias parsing lives inside S3 backend module | Move to `storage/credentials.rs` | S |
| 🟢 | code-quality | `crates/proteus-controller/src/backup/pvc_reader.rs:83` | Smell — `#[allow(clippy::too_many_arguments)]` on stream helper | Param struct `StreamPvcParams { ... }` | S |
| 🟢 | code-quality | `crates/proteus-core/src/storage/local.rs:226` vs `s3.rs:297` | DRY — `*.blob` filename → `ContentId` parse duplicated | `ContentId::from_blob_filename` | S |
| 🟢 | code-quality | `crates/proteus-ui/src/api.rs:5` vs `proteus-api/src/resources.rs:37` | Drift risk — UI/API DTO mirrors (WASM constraint) without shared types | Optional `proteus-api-types` or OpenAPI codegen later | L |

## GitHub tracking

Epic: [#49 Code-quality / SOLID debt](https://github.com/CraftingTech/proteus/issues/49)

| Finding focus | Issue |
| ------------- | ----- |
| 🔴 `delete_snapshot` / shared chunks | [#50](https://github.com/CraftingTech/proteus/issues/50) (coord. #29, #30) |
| 🟡 `skip_if_exists` + Local LSP | [#51](https://github.com/CraftingTech/proteus/issues/51) |
| 🟡 `resources.rs` god module | [#52](https://github.com/CraftingTech/proteus/issues/52) |
| 🟡 DRY open-repo / secrets / fingerprints / mount / upsert | [#53](https://github.com/CraftingTech/proteus/issues/53) |
| 🟡 `Result<_, String>` | [#54](https://github.com/CraftingTech/proteus/issues/54) |
| 🟡 UI `backups.rs` god page | [#55](https://github.com/CraftingTech/proteus/issues/55) |
| 🟢 minors + comment hygiene | [#56](https://github.com/CraftingTech/proteus/issues/56) |

## Top actions

1. **Neutralize `delete_snapshot` footgun** — [#50](https://github.com/CraftingTech/proteus/issues/50)
2. **Arm write-path CAS dedup + Local exclusive create** — [#51](https://github.com/CraftingTech/proteus/issues/51)
3. **Break god-modules + unify DRY clusters** — [#52](https://github.com/CraftingTech/proteus/issues/52), [#53](https://github.com/CraftingTech/proteus/issues/53), [#55](https://github.com/CraftingTech/proteus/issues/55)

## Coverage

- **Scanned**: code-quality
- **Skipped**: architecture, security, dependencies, performance, tests, ui (single-pillar run per user choice `1`)

### Scan evidence

| Area | Sampled |
| ---- | ------- |
| proteus-core | `storage/{traits,local,s3}`, `backup/pipeline`, `crypto`, `chunking`, `hash`, `error`, `lib` |
| proteus-controller | `controllers/*`, `backup/{mod,repo,pvc_reader}`, `restore/*`, `main`, `error` |
| proteus-api | `resources`, `routes`, `state`, `inventory`, `lib` |
| proteus-crd | structure / lib denies |
| proteus-ui | `api`, `pages/backups` (+ page sizes) |
| Searches | `skip_if_exists`, `PutOptions`, decode secret, fingerprint, unwrap/deny, line counts |

### Positive signals (not findings)

- `ObjectStore` trait + Local/S3 backends = solid **D** / **O** foundation
- Pipeline depends on `&dyn ObjectStore` (DIP respected in core)
- Libs use `thiserror`; no production `unwrap` found outside tests where denies apply

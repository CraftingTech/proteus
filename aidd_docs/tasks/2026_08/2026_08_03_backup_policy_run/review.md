# Review: backup policy / run split (#25)

- **Verdict**: changes-requested
- **Diff**: `main...feat/25-backup-policy-run`
- **Axes run**: code, functional, relevancy
- **Date**: 2026_08_03
- **Findings**: 0 critical, 4 warning, 2 minor

## Phases

### Phase 1 — CRD BackupPolicy + run policyRef

- [x] crdgen emits Policy CRD; Ready/Invalid status — `deploy/crds/proteusbackuppolicies.yaml`
- [x] Legacy sample Backup deserializes; policyRef-only Backup is valid typed object — `crates/proteus-crd/src/backup.rs` tests

### Phase 2 — Controller policy validate + resolve recipe

- [x] Editing Ready policy updates status only; no new Backup — `controllers/backup_policy.rs` (validate-only path)
- [x] Ready policyRef reaches run_backup; missing/Invalid policy → Failed; legacy inline still runs — `backup/recipe.rs`, `controllers/backup.rs`
- [ ] Policy with no status yet should not permanently Failed — currently terminal Failed (see Findings)

### Phase 3 — API policies CRUD + Run now

- [x] POST policy creates CR without Backup; GET/DELETE work — `resources/backup_policies.rs`, `routes.rs`
- [x] POST backup with policyRef creates run; restore still uses backup name / lastSnapshotId — `resources/backups.rs`

### Phase 4 — UI policies vs runs

- [x] Create policy → listed; Run now → run appears; restore from Succeeded run — `pages/backups.rs`
- [x] Policy edit via CR does not spawn runs (no UI edit; controller path OK)

## Findings

| Sev | Kind | Phase | Location | Issue | Fix |
| --- | ---- | ----- | -------- | ----- | --- |
| 🟡 | functional | 2 | `controllers/backup.rs` + `backup/recipe.rs:110-116` | Resolve fails with « no status yet » → terminal `Failed`. No policy→run requeue; Run now before first policy reconcile sticks forever. | Requeue (non-terminal) when policy status is absent; only Failed on Invalid/missing. Optionally API refuses create unless Ready. |
| 🟡 | code | 3 | `api/resources/repo_store.rs` keep-set + sample `policyRef`-only | GC keep-set matches `spec.repositoryRef` only. policyRef-only runs (empty repo ref) drop out of keep-set → snapshot purge risk when deleting another run. | Resolve recipe/repo for each keep candidate, or require stamped `repositoryRef` on all runs and document. |
| 🟡 | code | 2 | `controller/restore/mod.rs:25-31` | Restore opens `backup.spec.repositoryRef` without `resolve_recipe`. policyRef-only Succeeded run cannot restore. | Open repo via `resolve_recipe` (or stamped fields after reconcile copy). |
| 🟡 | code | 3 | `api/resources/backups.rs:220-227` | Delete GC prefers stamped `repositoryRef` while controller runs live policy. Policy repo change ⇒ GC wrong store / orphans. | Prefer live policy resolve for GC when `policyRef` set; stamp only for display. |
| 🟢 | conform | 3 | `api/resources/backups.rs` create path | API Run now does not require policy Ready (UI disables). | Align API with UI: 400 if not Ready. |
| 🟢 | rot | 4 | `ui/pages/backups.rs` | Page grew again (policies + runs + restore); still the #55 SRP debt. | Follow-up extract modules; not merge-blocking. |

## Verification

| Metric        | Value |
| ------------- | ----- |
| Verified      | 87% (7/8) |
| Files checked | `backup_policy.rs` (crd/controller/api), `backup/recipe.rs`, `controllers/backup.rs`, `resources/backups.rs`, `repo_store.rs`, `restore/mod.rs`, `pages/backups.rs`, CRD YAML, sample |
| Unchecked     | Phase 2 « no status yet » → fix |
| Unplanned     | Stamped recipe on run for list/GC (pragmatic; creates live-vs-stamp tension) |

# Review: backup policy / run split (#25)

- **Verdict**: approve
- **Diff**: `main...feat/25-backup-policy-run`
- **Axes run**: code, functional, relevancy
- **Date**: 2026_08_03
- **Findings**: 0 critical, 0 warning, 1 minor

## Phases

### Phase 1 — CRD BackupPolicy + run policyRef

- [x] crdgen emits Policy CRD; Ready/Invalid status — `deploy/crds/proteusbackuppolicies.yaml`
- [x] Legacy sample Backup deserializes; policyRef-only Backup is valid typed object — `crates/proteus-crd/src/backup.rs` tests

### Phase 2 — Controller policy validate + resolve recipe

- [x] Editing Ready policy updates status only; no new Backup — `controllers/backup_policy.rs`
- [x] Ready policyRef reaches run_backup; Invalid/missing → Failed; legacy inline still runs — `backup/recipe.rs`
- [x] Policy with no status yet → Pending + short requeue (not terminal Failed) — `controllers/backup.rs` + `ResolveRecipeError::NotReady`

### Phase 3 — API policies CRUD + Run now

- [x] POST policy creates CR without Backup; GET/DELETE work — `resources/backup_policies.rs`
- [x] POST backup with policyRef creates run; restore uses recipe resolve — `resources/backups.rs`, `restore/mod.rs`
- [x] Run now requires Ready; GC resolves live policy + keep-set for policyRef runs — `backups.rs`, `repo_store.rs`

### Phase 4 — UI policies vs runs

- [x] Create policy → listed; Run now → run appears; restore from Succeeded run — `pages/backups.rs`

## Findings

| Sev | Kind | Phase | Location | Issue | Fix |
| --- | ---- | ----- | -------- | ----- | --- |
| 🟢 | rot | 4 | `ui/pages/backups.rs` | Page still large (policies + runs + restore); #55 debt. | Follow-up extract; not merge-blocking. |

## Verification

| Metric        | Value |
| ------------- | ----- |
| Verified      | 100% (8/8) |
| Files checked | recipe resolve/load, backup reconcile, restore, API Ready gate, GC keep-set, UI |
| Unchecked     | none |
| Unplanned     | `load_recipe` for restore/GC vs `resolve_recipe` for run start (intentional) |

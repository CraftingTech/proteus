---
objective: "Operators manage idempotent backup policies and trigger runs without conflating recipe edits with executions; restore still resolves via ProteusBackup snapshot status."
status: implemented
---

# Plan: Split backup policy from backup run (#25)

## Overview

| Field      | Value |
| ---------- | ----- |
| **Goal**   | `ProteusBackupPolicy` (recipe) + `ProteusBackup` as run; UI policies + Run now |
| **Source** | GitHub #25 — enables #16 / #29; Launch v1 epic #68 |

## Phases

| #   | Phase | File |
| --- | ----- | ---- |
| 1   | CRD: BackupPolicy + run `policyRef` | [`phase-1.md`](./phase-1.md) |
| 2   | Controller: policy validate + resolve recipe | [`phase-2.md`](./phase-2.md) |
| 3   | API: policies CRUD + Run now | [`phase-3.md`](./phase-3.md) |
| 4   | UI: policies vs runs | [`phase-4.md`](./phase-4.md) |

## Resources

| Source | Verified |
| ------ | -------- |
| https://github.com/CraftingTech/proteus/issues/25 | Policy ≠ run; Run now; edit policy does not start backup |
| https://github.com/CraftingTech/proteus/issues/16 | Schedules must create runs from policies (follow-up) |
| https://github.com/CraftingTech/proteus/issues/68 | Launch v1 needs schedule UX built on this split |
| https://github.com/CraftingTech/proteus/pull/69 | PRD Launch v1 on main |

## Decisions

| Decision | Why |
| -------- | --- |
| New kind `ProteusBackupPolicy`; keep `ProteusBackup` as the **run** CR | Preserve `ProteusRestore.backupRef`, GC keep-set, metrics, UI run list with minimal churn |
| Recipe fields live on Policy (`repositoryRef`, `targetNamespace`, `pvcNames`, `retention`, dead `schedule`/`labelSelector` for later) | Single source of truth for “what”; #16/#29 attach here |
| Run may set `policyRef` **or** keep inline recipe for existing CRs (compat) | Alpha clusters already have inline `ProteusBackup`; no forced rewrite |
| New creates from API/UI always use Policy + run-with-`policyRef` | Forces the intended model going forward; schedules (#16) spawn runs the same way |
| Policy reconcile never calls `run_backup` | Issue QA: changing a policy must not start a backup |
| Retention fields move to Policy; run status keeps snapshot/metrics only | GC enforcement stays #29; structure ready now |

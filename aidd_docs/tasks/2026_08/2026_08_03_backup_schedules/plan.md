---
objective: "Operators schedule PVC backups on a ProteusBackupPolicy (cron + pause), see next/last run, observe failures as runs, and keepLast prunes old Succeeded runs so schedules do not grow the repo unbounded."
status: implemented
---

# Plan: Scheduled backups on BackupPolicy (#16 + keepLast #29)

## Overview

| Field      | Value |
| ---------- | ----- |
| **Goal**   | Cron on policy spawns runs; pause; next run UI; keepLast prune |
| **Source** | GitHub #16, #29 (keepLast only); Launch PRD / epic #68 |

## Phases

| #   | Phase | File |
| --- | ----- | ---- |
| 1   | CRD: pause + schedule status | [`phase-1.md`](./phase-1.md) |
| 2   | Controller: cron tick → spawn run | [`phase-2.md`](./phase-2.md) |
| 3   | Controller: keepLast prune + GC | [`phase-3.md`](./phase-3.md) |
| 4   | API patch + schedule fields | [`phase-4.md`](./phase-4.md) |
| 5   | UI: presets, pause, next run | [`phase-5.md`](./phase-5.md) |

## Resources

| Source | Verified |
| ------ | -------- |
| https://github.com/CraftingTech/proteus/issues/16 | Cron without manual trigger; enable/disable; failures visible |
| https://github.com/CraftingTech/proteus/issues/29 | keepLast prune of Succeeded runs + safe CAS GC |
| https://github.com/CraftingTech/proteus/issues/68 | Launch Must: schedules + retention minimum |
| https://github.com/CraftingTech/proteus/pull/71 | Policy/run split landed — spawn runs via policyRef |

## Decisions

| Decision | Why |
| -------- | --- |
| Schedule lives on `ProteusBackupPolicy`, not on the run CR | #25 model; schedules create runs |
| Add `spec.paused: bool` (default false) | Pause/disable without deleting cron |
| Status: `nextRunAt`, `lastScheduleTime`, `lastRunName` (RFC3339 / name) | UI + idempotent tick without inventing a Job CR |
| Crate `cron` (UTC) + existing `chrono` | Standard 5-field cron; no Node |
| Spawn = create `ProteusBackup` with `policyRef` (same as Run now naming) | One execution path |
| Skip spawn if a non-terminal run already exists for this policy | Avoid pile-up when previous run still Running |
| keepLast prune in this plan (TTL later) | Launch needs a retention floor; #29 can finish TTL later |
| UI: presets (hourly / daily 02:00 / weekly) + advanced cron text | PRD preference |

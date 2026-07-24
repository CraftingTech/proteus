---
objective: "Operators can restore any Succeeded backup's snapshot into target-namespace PVCs from the UI/API, with decryption when the repository key is set."
status: implemented
---

# Plan: M4 Restore any backup

## Overview

| Field      | Value |
| ---------- | ----- |
| **Goal**   | ProteusRestore engine + API create + UI flow; decrypt via repo Secret |
| **Source** | GitHub #10, #12 — parent #1 |

## Phases

| #   | Phase | File |
| --- | ----- | ---- |
| 1   | Core: load snapshot + materialize volume bytes | [`phase-1.md`](./phase-1.md) |
| 2   | Controller: run restore to Succeeded/Failed | [`phase-2.md`](./phase-2.md) |
| 3   | API create/list restore + RBAC PVC write | [`phase-3.md`](./phase-3.md) |
| 4   | UI: trigger restore + status | [`phase-4.md`](./phase-4.md) |

## Resources

| Source | Verified |
| ------ | -------- |
| https://github.com/CraftingTech/proteus/issues/10 | Restore any Succeeded backup → usable PVC |
| https://github.com/CraftingTech/proteus/issues/12 | UI primary path |
| https://github.com/CraftingTech/proteus/issues/1 | MVP loop closes with restore |

## Decisions

| Decision | Why |
| -------- | --- |
| Restore writes to PVCs named as in the snapshot, in `targetNamespace` (same names as backup) | Matches CRD; no rename mapping in MVP |
| Target PVC must already exist; if missing → Failed with clear message (no PVC create/resize in M4) | Avoid inventing storageClass/size; operator pre-provisions |
| `overwrite=false`: fail if `/data` is non-empty; `overwrite=true`: clear then untar | Safe default; matches CRD flag |
| Snapshot id = `spec.snapshotId` or backup `status.lastSnapshotId`; backup must be Succeeded when resolving from backup | Issue #10 |
| Reuse `open_repository` + decrypt when `manifest.encrypted`; mirror backup mount Pod with RW + `tar -xf -` | Proven M3 path |
| Terminal Succeeded/Failed + status fingerprint (no reconcile/pod flood) | Lesson from M3 |
| File-level browse restore out of scope | Project brief |

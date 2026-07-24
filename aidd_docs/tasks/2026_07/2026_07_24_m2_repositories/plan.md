---
objective: "Operators can create local and S3-compatible ProteusRepositories from the UI/API, see Ready/Failed status, and the CAS S3 backend can put/get objects."
status: implemented
---

# Plan: M2 Repositories (local + S3)

## Overview

| Field      | Value |
| ---------- | ----- |
| **Goal**   | CRUD repos from API/UI, honest Ready/Failed status, real S3 CAS I/O |
| **Source** | GitHub #4, #6, #7, #15 — parent #1 |

## Phases

| #   | Phase | File |
| --- | ----- | ---- |
| 1   | API CRUD for ProteusRepository | [`phase-1.md`](./phase-1.md) |
| 2   | Controller: validate + Ready/Failed status | [`phase-2.md`](./phase-2.md) |
| 3   | UI: create/list local + S3 repositories | [`phase-3.md`](./phase-3.md) |
| 4   | Real S3-compatible ObjectStore + reconcile probe | [`phase-4.md`](./phase-4.md) |

## Resources

| Source | Verified |
| ------ | -------- |
| https://github.com/CraftingTech/proteus/issues/4 | API CRUD |
| https://github.com/CraftingTech/proteus/issues/6 | UI create |
| https://github.com/CraftingTech/proteus/issues/7 | Status |
| https://github.com/CraftingTech/proteus/issues/15 | S3 CAS |

## Decisions

| Decision | Why |
| -------- | --- |
| Repos are namespaced CRs; API create takes `namespace` (default `proteus-system` or request body) | Matches sample resources / deploy namespace |
| Status uses existing `phase`/`message` (Ready/Failed); optional Condition later | CRD already has phase; avoid CRD bump in M2 unless needed |
| S3 credentials from Secret ref (`accessKeyId`/`secretAccessKey` or AWS-style keys) — document expected keys | CRD already has `credentialsSecretRef` |
| Full PVC backup through S3 is M3; M2 proves put/get + reconcile reachability | Keeps M2 shippable without inventing backup pipeline |
| UI: create form (local \| S3 toggle) + list with phase; edit/delete can be minimal (delete + recreate OK if update heavy) | MVP: create + list + delete; PATCH nice-to-have |

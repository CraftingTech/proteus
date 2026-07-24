---
objective: "Operators can see live Proteus CR counts, list CRs, browse cluster inventory (6 kinds) with filters, and get honest /readyz from the embedded UI."
status: implemented
---

# Plan: M1 Live control plane + cluster inventory

## Overview

| Field      | Value |
| ---------- | ----- |
| **Goal**   | Ship M1: live cluster snapshot, CR lists, inventory UI, strengthened readyz |
| **Source** | GitHub #1 (roadmap), #2, #3, #5, #17 — validated 2026-07-24 |

## Phases

| #   | Phase | File |
| --- | ----- | ---- |
| 1   | API: live state, CR lists, inventory, readyz | [`phase-1.md`](./phase-1.md) |
| 2   | UI: wire Cluster + Repositories/Backups/Restores lists | [`phase-2.md`](./phase-2.md) |
| 3   | UI: cluster inventory with filters | [`phase-3.md`](./phase-3.md) |

## Resources

| Source | Verified |
| ------ | -------- |
| https://github.com/CraftingTech/proteus/issues/1 | MVP decisions locked |
| https://github.com/CraftingTech/proteus/issues/17 | Inventory kinds + filters |

## Decisions

| Decision | Why |
| -------- | --- |
| API holds a kube `Client` (or thin inventory service) shared with handlers; controllers keep updating `ClusterSnapshot` counts | Snapshot alone cannot list objects or inventory |
| Inventory = `GET /api/v1/inventory?namespace=&kind=&q=` returning metadata-only rows | One endpoint keeps UI simple; secrets never include data values |
| UI fetch relative URLs (`/api/v1/...`) | Works with embedded UI on `:8080`; `just ui` alone will not hit API without proxy (document, do not block M1) |
| Nav: keep Cluster; add Inventory (or evolve PVCs page into Inventory); Restores via Backups page or dedicated list under existing IA | Minimal IA churn for M1 |

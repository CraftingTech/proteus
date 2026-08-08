---
objective: "PVC backup and restore move bulk data on the node via DaemonSet agent + mover Pods; exec remains fallback; status records dataPlane."
status: implemented
issue: https://github.com/CraftingTech/proteus/issues/66
---

# Plan: Node-agent MVP (backup + restore)

## Overview

| Field      | Value |
| ---------- | ----- |
| **Goal**   | Milestone B of #66: node-local backup **and** restore (same speed class), exec fallback |
| **Source** | Epic #66, ADR 0001 |

## Phases

| #   | Phase | File |
| --- | ----- | ---- |
| 1   | Binary modes + DaemonSet skeleton | [`phase-1.md`](./phase-1.md) |
| 2   | dataPlane status + plane selection | [`phase-2.md`](./phase-2.md) |
| 3   | Agent backup + mover ingest | [`phase-3.md`](./phase-3.md) |
| 4   | Agent restore + mover extract | [`phase-4.md`](./phase-4.md) |
| 5   | Docs + API/UI + unit tests | [`phase-5.md`](./phase-5.md) |

## Resources

| Source | Verified |
| ------ | -------- |
| https://github.com/CraftingTech/proteus/issues/66 | Epic milestones B–D |
| ADR 0001 | One image two modes; fallback matrix; mover Pods in scope |

## Decisions

| Decision | Why |
| -------- | --- |
| Mover Pod (PVC mounted) not hostPath in B1 | Node-local I/O without kubelet hostPath; ADR allows mover Pods |
| Restore in same milestone as backup | Product requires restore as fast as backup |
| Local emptyDir repo → force exec | Agent cannot reach controller emptyDir |
| Same GHCR image, `agent` / `mover` subcommands | ADR one-image packaging |

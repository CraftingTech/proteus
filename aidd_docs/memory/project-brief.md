# Project Brief

What this project is, the problem it solves, and its domain language. The non-derivable "why", not the "how".

## What it is

- **Proteus** is a lightweight, Kube-native backup and disaster-recovery system written in Rust
- It runs as a Kubernetes controller **with an embedded UI** so operators can drive backups without living in YAML alone
- Target users: cluster operators who want a simple control plane for backup destinations and jobs (Kopia-like UX, Kube-native runtime)

## Why it exists

- No off-the-shelf tool found that combines this product shape: **simple to deploy**, **Kube-native**, **own storage engine**, and a **usable UI** to configure repositories and run backups
- External / bolt-on backup stacks add operational weight; Proteus keeps CAS (dedup, chunking, encryption, compression) inside the operator and exposes day-2 ops through the UI
- Goal: durable, deduplicated backups with restore as a first-class CR — operable from the UI as the primary path

## Scope (MVP) — validated 2026-07-24

- **UI is required** (not a later nice-to-have)
- **Single Kubernetes cluster** only
- **Cluster inventory** in the UI: Deployments, Pods, Services, PVCs, ConfigMaps, Secrets (metadata only). Filters: namespace + kind + name search
- Backup destinations from the UI: **local AND S3-compatible** (both required)
- **Manual** PVC backup selection (no cron in MVP)
- **Encryption at rest** of backup data: **one key per Repository**, material in a Kubernetes Secret ref (no key rotation in MVP)
- **Restore any successful backup** to a target PVC/namespace — required for MVP to be useful

## Out of scope (MVP)

- Multi-cluster / cross-cluster migration
- Cron / scheduled PVC policies (tracked post-MVP)
- Inventory kinds beyond the v1 set (StatefulSets, Ingress, Jobs, …)
- File-level browse restore; compression; advanced retention UX; key rotation

## Launch v1 (product) — validated 2026-08-01

First release we call a **product** (post alpha). Full PRD: [`aidd_docs/tasks/2026_08/2026_08_01-proteus-launch-v1-prd.md`](../tasks/2026_08/2026_08_01-proteus-launch-v1-prd.md). GitHub epic: [#68](https://github.com/CraftingTech/proteus/issues/68).

**Must**
- Coherent brand + SRE-clear UI
- Easy cron/schedules (+ minimal retention)
- PVC filesystem backup/restore; snapshot-assisted when cluster allows
- ConfigMaps + Secrets backup/restore (secrets protected at rest)
- **One polished S3-compatible** destination UX (MinIO / AWS / R2 / Scaleway-class)
- Credible **~1 TiB** backup/restore (requires production data plane)

**Launch volume scope:** filesystem PVCs (RWO/RWX) + snapshots when available.

**Wanted later (not forgotten):** ephemeral / hostPath / local-PV edge cases; app-consistent hooks; native GCS / Azure / B2 / etc.; file-level browse restore.

## Domain language

| Term | Meaning |
| ---- | ------- |
| CAS | Content-addressable store; objects keyed by BLAKE3 content ID |
| Chunk | Fixed-size slice of a payload before hash / encrypt / store |
| Repository | Storage target (local path or S3-compatible) as a CR, set up from the UI |
| Snapshot | Immutable CAS root of a successful backup run |
| Reconcile | Controller loop that drives a CR toward its desired state |

## Key features

- Embedded UI: cluster inventory, repositories, manual backup/restore, status
- Custom Resources: `ProteusRepository`, `ProteusBackup`, `ProteusRestore` (`proteus.io/v1alpha1`)
- CAS engine with local + S3-compatible backends and encrypted payloads
- Kubernetes operator (`kube-rs` + Tokio) serving the Dioxus UI and API from the controller binary

## Roadmap

Tracked on GitHub: https://github.com/CraftingTech/proteus/issues/1

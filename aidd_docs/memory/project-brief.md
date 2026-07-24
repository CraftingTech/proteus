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

## Scope (MVP)

- **UI is required** for the MVP (not a later nice-to-have)
- Inventory and back up **PVCs** (list PVCs; backup flows centered on volume data)
- **Single Kubernetes cluster** only (no multi-cluster / cross-cluster migration in MVP)
- Backup destinations configurable from the UI: S3-compatible, local path / URL-style targets (repository setup)

## Domain language

| Term | Meaning |
| ---- | ------- |
| CAS | Content-addressable store; objects keyed by BLAKE3 content ID |
| Chunk | Fixed-size slice of a payload before hash / encrypt / store |
| Repository | Storage target (local path or S3-compatible) as a CR, set up from the UI |
| Snapshot | Immutable CAS root of a successful backup run |
| Reconcile | Controller loop that drives a CR toward its desired state |

## Key features

- Embedded UI to configure repositories, schedule/trigger backups, and inspect status (Kopia-inspired operator UX)
- Custom Resources: `ProteusRepository`, `ProteusBackup`, `ProteusRestore` (`proteus.io/v1alpha1`)
- CAS engine with local + S3-oriented backends
- PVC-centric backup inventory on one cluster
- Kubernetes operator (`kube-rs` + Tokio) serving the Dioxus UI and API from the controller binary

# Proteus — Launch v1 (product)

Proteus becomes a launchable Kube-native backup & restore product for DevOps/SRE: clear identity and control-plane UX, scheduled backups, durable volume + config/secret recovery, polished S3-compatible destinations, and confidence at ~1 TiB scale on a single cluster.

## Overview

Operators today either assemble heavy stacks (Velero + extras) or stick with incomplete MVP tooling. Proteus already has an early MVP (manual PVC backup/restore, local + S3-compatible repos, embedded UI). **Launch v1** is the first release we are willing to present as a product—not an alpha experiment.

**Primary persona:** cluster DevOps / SRE who lives in Kubernetes day-2 ops and wants backup/restore without living only in YAML.

**Validated product decisions (2026-08-01):**
- Volume scope for launch is focused; broader volume types are wanted later, not forgotten.
- Object storage for launch is a **single polished S3-compatible** experience (MinIO / AWS / R2 / Scaleway-class endpoints), not many native SDKs on day one.

## Problem Statement

SRE teams need a trustworthy path to protect application data and cluster configuration on one Kubernetes cluster, schedule it easily, restore under pressure, and store backups in the object stores they already pay for—without the product feeling unfinished (identity, UX, scale, or “it only works on tiny PVCs”).

Pain today with Proteus alpha: usable for demos/small volumes; not yet credible for ~1 TiB, schedules, config/secret DR, or a coherent operator-facing product surface.

## Goals

1. An SRE can recognize and operate Proteus from the UI as a backup control plane (brand + clear screens for inventory, repos, schedules, backup/restore status).
2. An SRE can set up a recurring PVC backup schedule in a few guided steps and see runs succeed/fail with clear status.
3. An SRE can back up and restore **filesystem PVCs** (RWO/RWX) used as app data volumes; when the cluster supports volume snapshots, they can use a snapshot-based path for better consistency.
4. An SRE can back up and restore **Kubernetes configuration objects including Secrets** (sensitive material protected at rest in the backup store).
5. An SRE can configure an **S3-compatible** repository (endpoint, region, credentials, bucket/prefix) with guided UX and docs that work for MinIO, AWS S3, Cloudflare R2, and Scaleway Object Storage without guessing.
6. On a representative cluster, Proteus can complete backup/restore of about **1 TiB** of PVC data within an operator-acceptable window (baseline + target recorded in release notes / QA), without controller OOM.
7. Launch is cut as a non-alpha product version (post `0.0.1-alpha.x`) with installable pinned release and public image.

## Non-Goals

- Multi-cluster / cross-cluster migration
- Native first-class SDKs for GCS, Azure Blob, B2, SFTP, WebDAV at launch (post-launch backlog)
- “Every” Kubernetes volume type at launch (ephemeral, hostPath, exotic local-PV edge cases — **wanted later**)
- App-consistent hooks / pre-post scripts at launch (wanted later)
- File-level browse / partial file restore at launch
- Key rotation UX at launch
- Helm chart (Kustomize remains the install path)
- Replacing the CAS model or leaving single-cluster scope

## User Stories

- As an SRE, I want a clear Proteus brand and operator UI, so that I can run backup/restore without reverse-engineering a placeholder shell.
- As an SRE, I want to schedule PVC backups simply, so that protection runs without manual clicks every night.
- As an SRE, I want to back up and restore application PVCs, so that stateful workloads survive loss or mistake.
- As an SRE, I want snapshot-assisted backup when my storage supports it, so that backups are more consistent under load.
- As an SRE, I want to back up and restore ConfigMaps and Secrets (and related manifests needed to reclaim apps), so that recovery is not data-only.
- As an SRE, I want to point Proteus at my S3-compatible bucket with clear fields and docs, so that MinIO/R2/Scaleway/AWS just work.
- As an SRE, I want Proteus to handle on the order of 1 TiB volumes, so that I can trust it beyond lab PVCs.
- As an SRE, I want predictable restore of a successful backup, so that DR drills succeed under time pressure.

## Acceptance Criteria

- [ ] UI shows a coherent product identity (not letter-mark placeholder) and SRE-clear flows for: cluster inventory, repositories, schedules, backup runs, restore.
- [ ] Cron/schedule: create, list, pause/disable, and observe resulting backup runs from the UI (and CR path).
- [ ] Retention minimum exists so schedules do not unbounded-grow the repository without operator intent (keep-last-N and/or TTL — product-visible).
- [ ] PVC filesystem backup + restore works for RWO and RWX as documented; status shows which data path ran when more than one exists (e.g. live vs snapshot-assisted).
- [ ] ConfigMaps and Secrets included in a backup/restore story with encrypted secret payloads at rest in the repository.
- [ ] S3-compatible repository setup validated against at least two providers from {MinIO, AWS S3, R2, Scaleway} with the same UX; docs cover endpoint/credential quirks.
- [ ] ~1 TiB backup completes without OOM; `durationSeconds` / throughput (or equivalent) recorded; restore of that backup verified on a test volume.
- [ ] Documented install from a pinned non-alpha release tag; GHCR image publicly pullable.
- [ ] Explicit “Later” backlog published for: ephemeral/hostPath/local edge volumes, app hooks, native GCS/Azure/etc.

## Dependencies

- Product decision: production data plane (node-local agent + snapshot-assisted path) accepted for scale — see epic #66 / ADR 0001 (merge #67).
- Visual identity assets: logo exists; design system / screen IA still needed (#57).
- Existing MVP capabilities (manual PVC backup/restore, local + S3-compatible repos) as baseline.
- Access to a CSI-capable cluster (or documented degrade) for snapshot acceptance; access to ~1 TiB test data for scale QA.
- GitHub epic [#68](https://github.com/CraftingTech/proteus/issues/68) tracks delivery against this PRD.

## Open Questions

- Exact numeric SLO for 1 TiB (e.g. max hours, min MiB/s) — set after first agent/CSI baselines.
- Launch version number (`0.1.0` vs `1.0.0-rc.1`) — decide when Must criteria are green.
- How much “related manifests” beyond ConfigMap/Secret is in launch vs later (#26 scope trim).
- Whether restore must **create** target PVCs at launch or only restore into pre-created PVCs (#31).
- Default schedule UX: calendar cron vs simplified presets (“daily 02:00”) — prefer presets + advanced cron.

---
objective: "Operators can manually trigger an encrypted PVC backup into a Ready repository from the UI/API and see Succeeded with a snapshot id."
status: implemented
---

# Plan: M3 Manual PVC backup + encryption

## Overview

| Field      | Value |
| ---------- | ----- |
| **Goal**   | Manual PVC → chunk → encrypt → CAS; UI create + status; one key/repo via Secret |
| **Source** | GitHub #8, #9, #13 — parent #1 |

## Phases

| #   | Phase | File |
| --- | ----- | ---- |
| 1   | CRD + encryption Secret for repositories | [`phase-1.md`](./phase-1.md) |
| 2   | Core: snapshot pipeline (chunk → encrypt → put) | [`phase-2.md`](./phase-2.md) |
| 3   | Controller: run backup to Succeeded/Failed | [`phase-3.md`](./phase-3.md) |
| 4   | API create backup + UI trigger/status | [`phase-4.md`](./phase-4.md) |

## Resources

| Source | Verified |
| ------ | -------- |
| https://github.com/CraftingTech/proteus/issues/8 | PVC → CAS encrypted, snapshot id |
| https://github.com/CraftingTech/proteus/issues/9 | UI pick PVC → Succeeded |
| https://github.com/CraftingTech/proteus/issues/13 | One key/repo in Secret; fail if missing |
| https://github.com/CraftingTech/proteus/issues/1 | MVP scope parent |

## Decisions

| Decision | Why |
| -------- | --- |
| Add `pvcNames: []string` on `ProteusBackup` (required, ≥1) for manual PVC pick; keep `labelSelector` optional | Issues demand manual PVC selection; labels alone are not enough |
| Encryption key = 32 raw bytes (or base64) in Secret keys `encryptionKey` / `ENCRYPTION_KEY`; API may generate + create Secret when `encryptionEnabled` | Matches #13; mirrors S3 creds UX |
| Chunk → BLAKE3 id on **plaintext** → AES-GCM encrypt → `put(id, ciphertext)`; snapshot manifest JSON stored as CAS object; `status.lastSnapshotId` = manifest ContentId hex | Dedup + restore-ready layout; uses existing crypto/CAS |
| Read PVC data via short-lived mount Pod + kube exec `tar` stream into controller (enable kube `ws`) | Works with `just run` (local controller); no in-cluster agent network path |
| Cron/`schedule` ignored in M3 (leave field; do not implement) | Post-MVP #16/#17 |
| Restore of snapshot = M4; M3 only needs encrypt-at-rest proof (blobs not plaintext) | Issue #13 restore bullet deferred to M4 except key load path reused |

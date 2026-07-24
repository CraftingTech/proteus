---
objective: "Large PVC backups complete without buffering the archive in controller RAM; duration and throughput are recorded on status for later measurement."
status: implemented
---

# Plan: M5 streaming backup (no agent image)

## Overview

| Field      | Value |
| ---------- | ----- |
| **Goal**   | Stream kube-exec `tar` → chunk → store (no full `Vec` buffer); expose duration/throughput |
| **Source** | User rejected in-cluster Job/image; keep `just run` workflow |

## Decisions

| Decision | Why |
| -------- | --- |
| No backup-agent Job / `PROTEUS_BACKUP_JOB_IMAGE` | Ops friction on Pi; user veto |
| Stream ingest via operator + mount Pod | Archivability without OOM; same `just run` path |
| Keep `durationSeconds` / `throughputBytesPerSec` | Measurement later |

## Note

An earlier draft used an in-cluster Job + custom image; that path was removed entirely.

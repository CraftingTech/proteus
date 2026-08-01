---
objective: "First concrete tranche of #24: trustworthy throughput baselines + hot-path stream ingest wins without a backup-agent image."
status: implemented
issue: https://github.com/CraftingTech/proteus/issues/24
---

# Plan: #24 backup stream throughput (tranche 1)

## Chosen optimization (why)

Keep mount-Pod + kube-exec `tar` (no agent Job/image). Evidence in `ingest_volume_stream`:

1. Unencrypted path copied every chunk (`Bytes::copy_from_slice`) while the read buffer was reused.
2. `store.put` was strictly sequential with the next read from the (often slow) apiserver exec stream.
3. Exec stdout was unbuffered; kube WebSocket frames are typically small.

Tranche 1 therefore:

- `BytesMut` → `freeze()` (drop extra memcpy on plaintext puts)
- pipeline depth 1: put chunk N ∥ read chunk N+1
- 256 KiB `BufReader` on exec stdout + 500 ms mount-Pod ready poll

## Metrics fix

`terminal_status` previously read `startedAt` from the stale reconcile `obj`, so first-run `durationSeconds` / `throughputBytesPerSec` were often missing. The Running patch's `startedAt` is now passed through explicitly; `compute_throughput` is unit-tested.

## How to measure (cluster)

Full e2e is not in CI. On a real cluster:

```bash
# After a successful ProteusBackup
kubectl get proteusbackup -n <ns> <name> -o jsonpath='{.status.lastBytes} {.status.durationSeconds} {.status.throughputBytesPerSec}{"\n"}'
```

Compare before/after on the same PVC size and repository backend (Local vs S3). Wall-clock includes mount-Pod pull/attach; for pure stream rate, prefer large PVCs where setup is amortized.

## Follow-ups (still #24)

- Apiserver hop remains the dominant ceiling for large volumes — node-local / CSI snapshot paths
- Deeper put concurrency (bounded window > 1) if S3 RTTs dominate after this slice
- Compression trade-offs vs dedup
- Explicit throughput SLO once baselines exist

---
verdict: ship
---

# Review: M4 Restore any backup

## Verdict

**ship** (after iterate: PVC existence pre-check)

## Iterate findings (resolved)

| Finding | Fix |
| ------- | --- |
| Missing target PVC waited 90s with generic Pod Pending error | `ensure_pvc_exists` in `restore/pvc_writer.rs` before mount Pod |

## Acceptance vs issues

| Issue | Met? | Notes |
| ----- | ---- | ----- |
| #10 Restore any Succeeded backup → PVC | yes | Core load/materialize + controller write path |
| #12 UI restore flow | yes | Form + poll + table |

## Residual risks

- Live e2e of busybox `find`/`rm`/`tar -xf` not automated
- Target PVC must be pre-created with enough capacity (documented)

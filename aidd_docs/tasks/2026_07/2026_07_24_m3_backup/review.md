---
verdict: ship
---

# Review: M3 Manual PVC backup + encryption

## Verdict

**ship** (after iterate fix for backup reconcile flood)

## Iterate findings (resolved)

| Finding | Fix |
| ------- | --- |
| Failed backup re-patched every reconcile (`last_failure_at` noise) → mount-pod flood | `status_changed` + terminal `Failed`/`Succeeded` short-circuit in `controllers/backup.rs` |
| `pvcNames` not constrained in CRD OpenAPI | `#[schemars(length(min = 1))]` + regenerate CRD |

## Acceptance vs issues

| Issue | Met? | Notes |
| ----- | ---- | ----- |
| #8 Backup PVC → CAS encrypted + snapshot id | yes | Pipeline + controller pod/exec path |
| #9 UI trigger + status | yes | Create form + poll |
| #13 Encryption via Secret | yes (backup path) | Restore same key = M4 (documented) |

## Residual risks

- Encrypted chunk put overwrites same ContentId with new nonce (weak dedup when encrypted)
- PATCH repo does not provision encryption Secret if toggled later
- Live e2e against a real PVC not automated in CI

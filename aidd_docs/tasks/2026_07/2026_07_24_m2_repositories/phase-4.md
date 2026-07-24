---
status: done
---

# Instruction: Real S3-compatible CAS backend

## Architecture projection

```txt
crates/proteus-core/src/storage/s3.rs   ✏️ real put/get/exists/delete
crates/proteus-core/Cargo.toml         ✏️ aws-sdk-s3 or equivalent
crates/proteus-controller/.../repository.rs  ✏️ use S3 backend for probe
tests or docs                          ✏️ MinIO smoke notes / unit test with mock if feasible
```

## Tasks to do

### `1)` Implement `ObjectStore` for S3

> put/get/exists/delete against S3-compatible endpoint; honour forcePathStyle, region, prefix, credentials.

### `2)` Wire reconcile probe

> Failed/Ready based on real client, not stub.

### `3)` Prove it

> Unit/integration test OR documented MinIO smoke (`just` recipe optional). At minimum: core tests for key layout / error mapping; skip live MinIO if no docker in CI.

## Test acceptance criteria

| Task | Acceptance criteria |
| ---- | ------------------- |
| 1 | `S3NotImplemented` gone from happy path; put then get round-trips (test or manual MinIO) |
| 2 | Repo with bad endpoint/secret → Failed; good MinIO config can → Ready |
| 3 | README or deploy note mentions MinIO-compatible settings |

## Note

Full backup/restore of PVC bytes through S3 is **M3**; this phase unlocks the storage primitive and status probe.

---
status: done
---

# Instruction: UI create local + S3 repositories

## Architecture projection

```txt
crates/proteus-ui/src/
  api.rs                 ✏️ create/delete repository clients
  pages/repositories.rs  ✏️ form + list + status + delete
  assets/styles.css      ✏️ form layout
```

## Wireframe

```txt
┌─ Repositories ─────────────────────────────────────┐
│ [ + New repository ]                               │
│                                                    │
│ Backend: (•) Local  ( ) S3                         │
│ Name [________]  Namespace [proteus-system ▾]      │
│ Local path [/var/lib/proteus/…]   or               │
│ Bucket / Endpoint / Region / Secret ref            │
│ [Create]                                           │
│────────────────────────────────────────────────────│
│ NAME     NS      BACKEND  PHASE   MESSAGE          │
│ local-1  proteus local    Ready   repository…   ✕  │
└────────────────────────────────────────────────────┘
```

## Tasks to do

### `1)` API client write

> POST create + DELETE; surface errors.

### `2)` Create form

> Toggle local/S3; required fields; submit → refresh list.

### `3)` List polish

> Show phase/message; delete action with confirm (simple window confirm OK).

## Test acceptance criteria

| Task | Acceptance criteria |
| ---- | ------------------- |
| 2 | Can create a local repo from UI; row appears with status |
| 2 | Can create an S3-shaped repo from UI (CR created; Ready depends on probe) |
| 3 | Delete removes the CR and row |

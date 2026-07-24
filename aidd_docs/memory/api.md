# API

The HTTP API surface: its style, the main resources, and the contracts.

## Style

- REST over HTTP via Axum, defined in `crates/proteus-api/src/routes.rs`
- Versioned JSON under `/api/v1/…`; probes and metrics at the root

## Resources

- `GET /healthz` — liveness
- `GET /readyz` — readiness (kube reachable + required Proteus CRDs present); non-200 when not ready
- `GET /api/v1/cluster` — `ClusterSnapshot` for the UI
- `GET /api/v1/repositories` — ProteusRepository list (name, namespace, phase, backend, message)
- `POST /api/v1/repositories` — create (default namespace `proteus-system`)
- `GET /api/v1/repositories/{namespace}/{name}` — get one
- `PATCH /api/v1/repositories/{namespace}/{name}` — patch description / encryption / backend
- `DELETE /api/v1/repositories/{namespace}/{name}` — delete
- `GET /api/v1/backups` — ProteusBackup list
- `GET /api/v1/restores` — ProteusRestore list
- `GET /api/v1/inventory?namespace=&kind=&q=` — cluster inventory metadata (Deployments, Pods, Services, PVCs, ConfigMaps, Secrets); Secret values never returned
- `GET /api/v1/namespaces` — namespace names for UI selectors
- `GET /metrics` — Prometheus text exposition (placeholder gauges)

## Contracts

- Errors as JSON `{ "error": "<message>" }` with appropriate HTTP status (`ApiError`)
- No OpenAPI document yet; types live in `proteus-api` (`ClusterSnapshot`, list/inventory DTOs, route handlers)
- Listen address from `PROTEUS_API_ADDR` (default `0.0.0.0:8080`)
- UI uses relative `/api/v1/...` URLs (embedded on `:8080`)

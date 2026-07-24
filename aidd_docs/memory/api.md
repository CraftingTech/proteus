# API

The HTTP API surface: its style, the main resources, and the contracts.

## Style

- REST over HTTP via Axum, defined in `crates/proteus-api/src/routes.rs`
- Versioned JSON under `/api/v1/…`; probes and metrics at the root

## Resources

- `GET /healthz` — liveness
- `GET /readyz` — readiness (snapshot lock readable)
- `GET /api/v1/cluster` — `ClusterSnapshot` for the UI
- `GET /metrics` — Prometheus text exposition (placeholder gauges)

## Contracts

- Errors as JSON `{ "error": "<message>" }` with appropriate HTTP status (`ApiError`)
- No OpenAPI document yet; types live in `proteus-api` (`ClusterSnapshot`, route handlers)
- Listen address from `PROTEUS_API_ADDR` (default `0.0.0.0:8080`)

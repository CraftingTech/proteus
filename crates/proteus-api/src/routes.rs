use axum::extract::State;
use axum::routing::get;
use axum::{Json, Router};
use serde::Serialize;
use tower_http::trace::TraceLayer;

use crate::error::ApiResult;
use crate::state::{ApiState, ClusterSnapshot};
use crate::ui::static_handler;

pub fn router(state: ApiState) -> Router {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/readyz", get(readyz))
        .route("/api/v1/cluster", get(cluster_state))
        .route("/metrics", get(metrics_placeholder))
        .fallback(static_handler)
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

#[derive(Serialize)]
struct HealthBody {
    status: &'static str,
}

async fn healthz() -> Json<HealthBody> {
    Json(HealthBody { status: "ok" })
}

async fn readyz(State(state): State<ApiState>) -> Json<HealthBody> {
    let _guard = state.snapshot.read();
    drop(_guard);
    Json(HealthBody { status: "ready" })
}

async fn cluster_state(State(state): State<ApiState>) -> ApiResult<Json<ClusterSnapshot>> {
    Ok(Json(state.snapshot.read().clone()))
}

async fn metrics_placeholder(State(state): State<ApiState>) -> String {
    let snap = state.snapshot.read();
    format!(
        "# HELP proteus_repositories Number of ProteusRepository objects\n\
         # TYPE proteus_repositories gauge\n\
         proteus_repositories {}\n\
         # HELP proteus_backups Number of ProteusBackup objects\n\
         # TYPE proteus_backups gauge\n\
         proteus_backups {}\n\
         # HELP proteus_restores Number of ProteusRestore objects\n\
         # TYPE proteus_restores gauge\n\
         proteus_restores {}\n",
        snap.repositories, snap.backups, snap.restores
    )
}

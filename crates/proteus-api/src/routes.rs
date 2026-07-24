use axum::extract::{Query, State};
use axum::http::{HeaderValue, Method, StatusCode};
use axum::response::IntoResponse;
use axum::routing::get;
use axum::{Json, Router};
use serde::Serialize;
use tower_http::cors::{Any, CorsLayer};
use tower_http::trace::TraceLayer;

use crate::error::ApiResult;
use crate::inventory::{list_inventory, InventoryItem, InventoryQuery};
use crate::namespaces::{list_namespaces, NamespaceItem};
use crate::resources::{
    list_backups, list_repositories, list_restores, BackupListItem, RepositoryListItem,
    RestoreListItem,
};
use crate::state::{ApiState, ClusterSnapshot};
use crate::ui::static_handler;

pub fn router(state: ApiState) -> Router {
    // Allow `just ui` (dx on :5173) to call the controller API on :8080.
    let cors = CorsLayer::new()
        .allow_origin([
            "http://127.0.0.1:5173".parse::<HeaderValue>().expect("origin"),
            "http://localhost:5173".parse::<HeaderValue>().expect("origin"),
        ])
        .allow_methods([Method::GET, Method::POST, Method::PUT, Method::PATCH, Method::DELETE])
        .allow_headers(Any);

    Router::new()
        .route("/healthz", get(healthz))
        .route("/readyz", get(readyz))
        .route("/api/v1/cluster", get(cluster_state))
        .route("/api/v1/repositories", get(repositories))
        .route("/api/v1/backups", get(backups))
        .route("/api/v1/restores", get(restores))
        .route("/api/v1/inventory", get(inventory))
        .route("/api/v1/namespaces", get(namespaces))
        .route("/metrics", get(metrics_placeholder))
        .fallback(static_handler)
        .layer(cors)
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

async fn readyz(State(state): State<ApiState>) -> impl IntoResponse {
    state.refresh_readiness().await;
    if state.is_ready() {
        (StatusCode::OK, Json(HealthBody { status: "ready" })).into_response()
    } else {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(HealthBody {
                status: "not ready",
            }),
        )
            .into_response()
    }
}

async fn cluster_state(State(state): State<ApiState>) -> ApiResult<Json<ClusterSnapshot>> {
    Ok(Json(state.snapshot.read().clone()))
}

async fn repositories(State(state): State<ApiState>) -> ApiResult<Json<Vec<RepositoryListItem>>> {
    Ok(Json(list_repositories(&state).await?))
}

async fn backups(State(state): State<ApiState>) -> ApiResult<Json<Vec<BackupListItem>>> {
    Ok(Json(list_backups(&state).await?))
}

async fn restores(State(state): State<ApiState>) -> ApiResult<Json<Vec<RestoreListItem>>> {
    Ok(Json(list_restores(&state).await?))
}

async fn inventory(
    State(state): State<ApiState>,
    Query(query): Query<InventoryQuery>,
) -> ApiResult<Json<Vec<InventoryItem>>> {
    Ok(Json(list_inventory(&state, &query).await?))
}

async fn namespaces(State(state): State<ApiState>) -> ApiResult<Json<Vec<NamespaceItem>>> {
    Ok(Json(list_namespaces(&state).await?))
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

#[cfg(test)]
mod tests {
    use crate::state::Readiness;

    #[test]
    fn readiness_requires_kube_and_crds() {
        assert!(!Readiness {
            kube_reachable: false,
            crds_ready: true,
        }
        .is_ready());
        assert!(!Readiness {
            kube_reachable: true,
            crds_ready: false,
        }
        .is_ready());
        assert!(Readiness {
            kube_reachable: true,
            crds_ready: true,
        }
        .is_ready());
    }
}

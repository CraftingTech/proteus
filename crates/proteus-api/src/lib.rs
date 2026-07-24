//! HTTP API and embedded UI for the Proteus controller.

#![deny(clippy::unwrap_used)]
#![deny(clippy::expect_used)]
#![deny(clippy::panic)]

mod error;
mod routes;
mod state;
mod ui;

pub use error::{ApiError, ApiResult};
pub use routes::router;
pub use state::{ApiState, ClusterSnapshot};

use std::net::SocketAddr;

use axum::Router;
use tokio::net::TcpListener;
use tracing::info;

pub async fn serve(addr: SocketAddr, app: Router) -> ApiResult<()> {
    let listener = TcpListener::bind(addr)
        .await
        .map_err(|source| ApiError::Bind { addr, source })?;
    info!(%addr, "proteus-api listening");
    axum::serve(listener, app)
        .await
        .map_err(|source| ApiError::Serve { source })?;
    Ok(())
}

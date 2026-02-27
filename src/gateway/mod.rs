pub mod ws;

use std::sync::Arc;

use anyhow::Result;
use axum::{
    http::StatusCode,
    response::{Html, IntoResponse},
    routing::get,
    Router,
};
use tower_http::timeout::TimeoutLayer;

use crate::agent::Agent;

// ── AppState ────────────────────────────────────────────────────────────────

/// Shared application state passed to all handlers.
#[derive(Clone)]
pub struct AppState {
    pub agent: Arc<Agent>,
}

// ── Static content ──────────────────────────────────────────────────────────

/// The web UI, embedded at compile time.
const INDEX_HTML: &str = include_str!("../../web/index.html");

/// Serve the web UI.
async fn index_handler() -> Html<&'static str> {
    Html(INDEX_HTML)
}

/// Health check endpoint.
async fn health_handler() -> impl IntoResponse {
    (StatusCode::OK, "ok")
}

// ── Router & server ─────────────────────────────────────────────────────────

/// Build the Axum router with all routes.
fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/", get(index_handler))
        .route("/health", get(health_handler))
        .route("/ws", get(ws::ws_handler))
        .layer(TimeoutLayer::with_status_code(
            StatusCode::REQUEST_TIMEOUT,
            std::time::Duration::from_secs(30),
        ))
        .with_state(state)
}

/// Start the HTTP server.
pub async fn serve(host: &str, port: u16, agent: Arc<Agent>) -> Result<()> {
    let state = AppState { agent };
    let app = build_router(state);

    let addr = format!("{}:{}", host, port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!("Listening on {}", addr);

    axum::serve(listener, app).await?;

    Ok(())
}

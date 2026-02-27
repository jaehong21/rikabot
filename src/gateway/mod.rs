pub mod ws;

use std::sync::Arc;

use anyhow::Result;
use axum::{
    http::StatusCode,
    response::IntoResponse,
    routing::{get, get_service},
    Router,
};
use tower_http::{
    services::{ServeDir, ServeFile},
    timeout::TimeoutLayer,
};

use crate::agent::Agent;
use crate::session::SessionManager;

// ── AppState ────────────────────────────────────────────────────────────────

/// Shared application state passed to all handlers.
#[derive(Clone)]
pub struct AppState {
    pub agent: Arc<Agent>,
    pub sessions: Arc<tokio::sync::Mutex<SessionManager>>,
}

/// Health check endpoint.
async fn health_handler() -> impl IntoResponse {
    (StatusCode::OK, "ok")
}

// ── Router & server ─────────────────────────────────────────────────────────

/// Build the Axum router with all routes.
fn build_router(state: AppState) -> Router {
    let web_service =
        get_service(ServeDir::new("web/dist").fallback(ServeFile::new("web/dist/index.html")));

    Router::new()
        .route("/health", get(health_handler))
        .route("/ws", get(ws::ws_handler))
        .fallback_service(web_service)
        .layer(TimeoutLayer::with_status_code(
            StatusCode::REQUEST_TIMEOUT,
            std::time::Duration::from_secs(30),
        ))
        .with_state(state)
}

/// Start the HTTP server.
pub async fn serve(
    host: &str,
    port: u16,
    agent: Arc<Agent>,
    sessions: Arc<tokio::sync::Mutex<SessionManager>>,
) -> Result<()> {
    let state = AppState { agent, sessions };
    let app = build_router(state);

    let addr = format!("{}:{}", host, port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!("Listening on http://{}", addr);

    axum::serve(listener, app).await?;

    Ok(())
}

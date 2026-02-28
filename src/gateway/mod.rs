pub mod ws;

use std::sync::Arc;

use anyhow::Result;
use axum::{
    http::StatusCode,
    response::IntoResponse,
    routing::{get, get_service},
    Router,
};
use serde_json::Value;
use tokio::{sync::mpsc, task::JoinHandle};
use tower_http::{
    services::{ServeDir, ServeFile},
    timeout::TimeoutLayer,
};

use crate::agent::Agent;
use crate::config::PermissionsConfig;
use crate::config_store::ConfigStore;
use crate::mcp_runtime::McpRuntime;
use crate::permissions::PermissionEngine;
use crate::prompt::PromptManager;
use crate::session::SessionManager;

// ── AppState ────────────────────────────────────────────────────────────────

pub struct ActiveRunState {
    pub run_id: u64,
    pub session_id: String,
    pub events: Vec<Value>,
    pub subscribers: Vec<mpsc::UnboundedSender<Value>>,
    pub agent_task: JoinHandle<()>,
    pub event_task: JoinHandle<()>,
}

pub struct RunManager {
    pub next_run_id: u64,
    pub active: Option<ActiveRunState>,
}

impl Default for RunManager {
    fn default() -> Self {
        Self {
            next_run_id: 1,
            active: None,
        }
    }
}

/// Shared application state passed to all handlers.
#[derive(Clone)]
pub struct AppState {
    pub agent: Arc<Agent>,
    pub sessions: Arc<tokio::sync::Mutex<SessionManager>>,
    pub prompt_manager: Arc<PromptManager>,
    pub runs: Arc<tokio::sync::Mutex<RunManager>>,
    pub permissions_config: Arc<tokio::sync::RwLock<PermissionsConfig>>,
    pub permission_engine: Arc<tokio::sync::RwLock<PermissionEngine>>,
    pub config_store: Arc<ConfigStore>,
    pub mcp_runtime: Arc<McpRuntime>,
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
    prompt_manager: Arc<PromptManager>,
    permissions_config: Arc<tokio::sync::RwLock<PermissionsConfig>>,
    permission_engine: Arc<tokio::sync::RwLock<PermissionEngine>>,
    config_store: Arc<ConfigStore>,
    mcp_runtime: Arc<McpRuntime>,
) -> Result<()> {
    let state = AppState {
        agent,
        sessions,
        prompt_manager,
        runs: Arc::new(tokio::sync::Mutex::new(RunManager::default())),
        permissions_config,
        permission_engine,
        config_store,
        mcp_runtime,
    };
    let app = build_router(state);

    let addr = format!("{}:{}", host, port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!("Listening on http://{}", addr);

    axum::serve(listener, app).await?;

    Ok(())
}

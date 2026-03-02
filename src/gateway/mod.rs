pub mod ws;

use std::collections::HashSet;
use std::sync::Arc;

use anyhow::Result;
use axum::{
    body::Body,
    http::{header, StatusCode, Uri},
    response::{IntoResponse, Response},
    routing::get,
    Router,
};
use mime_guess::from_path;
use rust_embed::{EmbeddedFile, RustEmbed};
use serde_json::Value;
use tokio::{sync::mpsc, task::JoinHandle};
use tower_http::timeout::TimeoutLayer;

use crate::agent::Agent;
use crate::agent::ToolApprovalDecision;
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
    pub approval_tx: mpsc::UnboundedSender<ToolApprovalDecision>,
    pub pending_approval_ids: HashSet<String>,
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

#[derive(RustEmbed)]
#[folder = "web/dist/"]
struct WebAssets;

fn static_file_response(path: &str, file: EmbeddedFile) -> Response {
    let mime = from_path(path).first_or_octet_stream();
    (
        [(header::CONTENT_TYPE, mime.as_ref())],
        Body::from(file.data.into_owned()),
    )
        .into_response()
}

fn is_asset_path(path: &str) -> bool {
    path.rsplit('/')
        .next()
        .is_some_and(|segment| segment.contains('.'))
}

async fn static_handler(uri: Uri) -> Response {
    let trimmed_path = uri.path().trim_start_matches('/');
    let path = if trimmed_path.is_empty() {
        "index.html"
    } else {
        trimmed_path
    };

    if let Some(file) = WebAssets::get(path) {
        return static_file_response(path, file);
    }

    if !is_asset_path(path) {
        if let Some(index) = WebAssets::get("index.html") {
            return static_file_response("index.html", index);
        }
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            "embedded web assets are missing index.html",
        )
            .into_response();
    }

    (StatusCode::NOT_FOUND, "not found").into_response()
}

// ── Router & server ─────────────────────────────────────────────────────────

/// Build the Axum router with all routes.
fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health_handler))
        .route("/ws", get(ws::ws_handler))
        .fallback(get(static_handler))
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

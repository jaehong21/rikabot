use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::Serialize;
use tokio::sync::watch;

use crate::config::McpServerConfig;
use crate::tools::mcp_client::{McpRegistry, McpServer};
use crate::tools::ToolRegistry;

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum McpServerState {
    Pending,
    Connecting,
    Ready,
    Failed,
    Disabled,
}

#[derive(Debug, Clone, Serialize)]
pub struct McpToolStatus {
    pub name: String,
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct McpServerStatus {
    pub name: String,
    pub state: McpServerState,
    pub tool_count: usize,
    pub tools: Vec<McpToolStatus>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct McpStatusSnapshot {
    pub enabled: bool,
    pub servers: Vec<McpServerStatus>,
}

#[derive(Clone)]
pub struct McpRuntime {
    inner: Arc<McpRuntimeInner>,
}

struct McpRuntimeInner {
    enabled: bool,
    order: Vec<String>,
    statuses: Mutex<HashMap<String, McpServerStatus>>,
    tx: watch::Sender<McpStatusSnapshot>,
}

impl McpRuntime {
    pub fn new(enabled: bool, servers: &[McpServerConfig]) -> Self {
        let mut order = Vec::with_capacity(servers.len());
        let mut statuses = HashMap::with_capacity(servers.len());

        for server in servers {
            order.push(server.name.clone());
            let state = if !enabled || !server.enabled {
                McpServerState::Disabled
            } else {
                McpServerState::Pending
            };
            statuses.insert(
                server.name.clone(),
                McpServerStatus {
                    name: server.name.clone(),
                    state,
                    tool_count: 0,
                    tools: Vec::new(),
                    error: None,
                },
            );
        }

        let snapshot = build_snapshot(enabled, &order, &statuses);
        let (tx, _rx) = watch::channel(snapshot);

        Self {
            inner: Arc::new(McpRuntimeInner {
                enabled,
                order,
                statuses: Mutex::new(statuses),
                tx,
            }),
        }
    }

    pub fn snapshot(&self) -> McpStatusSnapshot {
        self.inner.tx.borrow().clone()
    }

    pub fn subscribe(&self) -> watch::Receiver<McpStatusSnapshot> {
        self.inner.tx.subscribe()
    }

    pub fn spawn_background(
        &self,
        configs: Vec<McpServerConfig>,
        workspace_dir: PathBuf,
        tool_registry: ToolRegistry,
    ) {
        if !self.inner.enabled || configs.is_empty() {
            return;
        }

        for config in configs {
            if !config.enabled {
                continue;
            }

            let runtime = self.clone();
            let workspace = workspace_dir.clone();
            let mut registry = tool_registry.clone();

            tokio::spawn(async move {
                runtime
                    .connect_server_with_retry(config, workspace, &mut registry)
                    .await;
            });
        }
    }

    async fn connect_server_with_retry(
        &self,
        config: McpServerConfig,
        workspace_dir: PathBuf,
        tool_registry: &mut ToolRegistry,
    ) {
        let mut attempt: u32 = 0;

        loop {
            self.set_status(
                &config.name,
                McpServerState::Connecting,
                0,
                Vec::new(),
                None,
            );

            match McpServer::connect(config.clone(), &workspace_dir).await {
                Ok(server) => {
                    let tools = server.tools().await;
                    let tool_statuses: Vec<McpToolStatus> = tools
                        .iter()
                        .map(|tool| McpToolStatus {
                            name: tool.name.clone(),
                            description: tool.description.clone(),
                        })
                        .collect();
                    let registry = Arc::new(McpRegistry::from_server(&config.name, server).await);
                    match tool_registry.register_mcp_tools(registry).await {
                        Ok(added) => {
                            self.set_status(
                                &config.name,
                                McpServerState::Ready,
                                added,
                                tool_statuses,
                                None,
                            );
                            tracing::info!(
                                "MCP server `{}` ready ({} tool{})",
                                config.name,
                                added,
                                if added == 1 { "" } else { "s" }
                            );
                            return;
                        }
                        Err(err) => {
                            attempt = attempt.saturating_add(1);
                            let message =
                                format!("failed to register tools for `{}`: {}", config.name, err);
                            self.set_status(
                                &config.name,
                                McpServerState::Failed,
                                0,
                                Vec::new(),
                                Some(message.clone()),
                            );
                            tracing::error!("{}", message);
                        }
                    }
                }
                Err(err) => {
                    attempt = attempt.saturating_add(1);
                    let message = err.to_string();
                    self.set_status(
                        &config.name,
                        McpServerState::Failed,
                        0,
                        Vec::new(),
                        Some(message.clone()),
                    );
                    tracing::warn!(
                        "failed to connect MCP server `{}`: {}",
                        config.name,
                        message
                    );
                }
            }

            let wait_secs = retry_delay_secs(attempt);
            tracing::info!(
                "Retrying MCP server `{}` in {}s (attempt #{})",
                config.name,
                wait_secs,
                attempt.saturating_add(1)
            );
            tokio::time::sleep(Duration::from_secs(wait_secs)).await;
        }
    }

    fn set_status(
        &self,
        name: &str,
        state: McpServerState,
        tool_count: usize,
        tools: Vec<McpToolStatus>,
        error: Option<String>,
    ) {
        let snapshot = {
            let mut statuses = match self.inner.statuses.lock() {
                Ok(guard) => guard,
                Err(_) => {
                    tracing::error!("MCP runtime status mutex poisoned");
                    return;
                }
            };

            let entry = statuses.entry(name.to_string()).or_insert(McpServerStatus {
                name: name.to_string(),
                state: McpServerState::Pending,
                tool_count: 0,
                tools: Vec::new(),
                error: None,
            });
            entry.state = state;
            entry.tool_count = tool_count;
            entry.tools = tools;
            entry.error = error;

            build_snapshot(self.inner.enabled, &self.inner.order, &statuses)
        };

        let _ = self.inner.tx.send(snapshot);
    }
}

fn build_snapshot(
    enabled: bool,
    order: &[String],
    statuses: &HashMap<String, McpServerStatus>,
) -> McpStatusSnapshot {
    let mut servers = Vec::with_capacity(statuses.len());
    for name in order {
        if let Some(status) = statuses.get(name) {
            servers.push(status.clone());
        }
    }
    for (name, status) in statuses {
        if !order.iter().any(|existing| existing == name) {
            servers.push(status.clone());
        }
    }

    McpStatusSnapshot { enabled, servers }
}

fn retry_delay_secs(attempt: u32) -> u64 {
    match attempt {
        0 => 1,
        1 => 2,
        2 => 4,
        3 => 8,
        4 => 15,
        _ => 30,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_marks_enabled_servers_pending() {
        let cfg = McpServerConfig {
            name: "linear".to_string(),
            enabled: true,
            transport: crate::config::McpTransport::Http,
            auth_mode: crate::config::McpAuthMode::Headers,
            command: None,
            args: vec![],
            env: HashMap::new(),
            cwd: None,
            url: Some("https://mcp.linear.app/mcp".to_string()),
            headers: HashMap::new(),
            oauth_client_id: None,
            oauth_client_secret_env: None,
            oauth_scopes: vec![],
            oauth_authorization_server: None,
            tool_timeout_secs: None,
            init_timeout_secs: None,
        };
        let runtime = McpRuntime::new(true, &[cfg]);
        let snapshot = runtime.snapshot();
        assert!(snapshot.enabled);
        assert_eq!(snapshot.servers.len(), 1);
        assert_eq!(snapshot.servers[0].state, McpServerState::Pending);
    }

    #[test]
    fn retry_delay_is_capped() {
        assert_eq!(retry_delay_secs(0), 1);
        assert_eq!(retry_delay_secs(1), 2);
        assert_eq!(retry_delay_secs(2), 4);
        assert_eq!(retry_delay_secs(3), 8);
        assert_eq!(retry_delay_secs(4), 15);
        assert_eq!(retry_delay_secs(9), 30);
    }
}

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use anyhow::{anyhow, bail, Context, Result};
use serde_json::json;
use tokio::sync::Mutex;

use crate::config::McpServerConfig;
use crate::tools::mcp_protocol::{
    JsonRpcRequest, McpToolDef, McpToolsListResult, MCP_PROTOCOL_VERSION,
};
use crate::tools::mcp_transport::{create_transport, McpTransportConn};

struct McpServerInner {
    config: McpServerConfig,
    transport: Box<dyn McpTransportConn>,
    next_id: AtomicU64,
    tools: Vec<McpToolDef>,
}

#[derive(Clone)]
pub struct McpServer {
    inner: Arc<Mutex<McpServerInner>>,
}

impl McpServer {
    pub async fn connect(config: McpServerConfig) -> Result<Self> {
        let mut transport = create_transport(&config)
            .with_context(|| format!("failed to create MCP transport for `{}`", config.name))?;

        let init_req = JsonRpcRequest::new(
            1,
            "initialize",
            json!({
                "protocolVersion": MCP_PROTOCOL_VERSION,
                "capabilities": {},
                "clientInfo": {
                    "name": "rikabot",
                    "version": env!("CARGO_PKG_VERSION"),
                }
            }),
        );
        let init_timeout = config.resolved_init_timeout_secs();
        let init_resp = transport.send_and_recv(&init_req, init_timeout).await?;
        if let Some(err) = init_resp.error {
            bail!("initialize failed: {} ({})", err.message, err.code);
        }

        let notif = JsonRpcRequest::notification("notifications/initialized", json!({}));
        let _ = transport.send_and_recv(&notif, init_timeout).await;

        let list_req = JsonRpcRequest::new(2, "tools/list", json!({}));
        let list_resp = transport.send_and_recv(&list_req, init_timeout).await?;
        if let Some(err) = list_resp.error {
            bail!("tools/list failed: {} ({})", err.message, err.code);
        }
        let result = list_resp
            .result
            .ok_or_else(|| anyhow!("tools/list returned no result"))?;
        let tool_list: McpToolsListResult =
            serde_json::from_value(result).context("failed to parse tools/list result shape")?;

        let inner = McpServerInner {
            config,
            transport,
            next_id: AtomicU64::new(3),
            tools: tool_list.tools,
        };

        Ok(Self {
            inner: Arc::new(Mutex::new(inner)),
        })
    }

    pub async fn tools(&self) -> Vec<McpToolDef> {
        self.inner.lock().await.tools.clone()
    }

    pub async fn call_tool(
        &self,
        tool_name: &str,
        arguments: serde_json::Value,
    ) -> Result<serde_json::Value> {
        let mut inner = self.inner.lock().await;
        let id = inner.next_id.fetch_add(1, Ordering::Relaxed);
        let req = JsonRpcRequest::new(
            id,
            "tools/call",
            json!({
                "name": tool_name,
                "arguments": arguments
            }),
        );

        let timeout_secs = inner.config.resolved_tool_timeout_secs();
        let resp = inner.transport.send_and_recv(&req, timeout_secs).await?;
        if let Some(err) = resp.error {
            bail!(
                "MCP tool `{}` error {}: {}",
                tool_name,
                err.code,
                err.message
            );
        }
        Ok(resp.result.unwrap_or(serde_json::Value::Null))
    }
}

pub struct McpRegistry {
    servers: Vec<McpServer>,
    tool_index: HashMap<String, (usize, String)>,
}

impl McpRegistry {
    pub async fn connect_all(configs: &[McpServerConfig]) -> Self {
        let mut servers = Vec::new();
        let mut tool_index = HashMap::new();

        for config in configs {
            if !config.enabled {
                tracing::info!("MCP server `{}` disabled; skipping", config.name);
                continue;
            }

            match McpServer::connect(config.clone()).await {
                Ok(server) => {
                    let server_idx = servers.len();
                    let tools = server.tools().await;
                    for tool in tools {
                        let prefixed = format!("{}__{}", config.name, tool.name);
                        tool_index.insert(prefixed, (server_idx, tool.name));
                    }
                    tracing::info!(
                        "MCP server `{}` connected ({} tools)",
                        config.name,
                        tool_index
                            .keys()
                            .filter(|k| k.starts_with(&format!("{}__", config.name)))
                            .count()
                    );
                    servers.push(server);
                }
                Err(e) => {
                    tracing::error!("failed to connect MCP server `{}`: {:#}", config.name, e);
                }
            }
        }

        Self {
            servers,
            tool_index,
        }
    }

    pub fn tool_names(&self) -> Vec<String> {
        self.tool_index.keys().cloned().collect()
    }

    pub async fn get_tool_def(&self, prefixed_name: &str) -> Option<McpToolDef> {
        let (server_idx, original_name) = self.tool_index.get(prefixed_name)?;
        let inner = self.servers[*server_idx].inner.lock().await;
        inner
            .tools
            .iter()
            .find(|t| t.name == *original_name)
            .cloned()
    }

    pub async fn call_tool(
        &self,
        prefixed_name: &str,
        arguments: serde_json::Value,
    ) -> Result<String> {
        let (server_idx, original_name) = self
            .tool_index
            .get(prefixed_name)
            .ok_or_else(|| anyhow!("unknown MCP tool `{}`", prefixed_name))?;
        let result = self.servers[*server_idx]
            .call_tool(original_name, arguments)
            .await?;
        serde_json::to_string_pretty(&result)
            .context("failed to serialize MCP tool result as pretty JSON")
    }

    pub fn is_empty(&self) -> bool {
        self.tool_index.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::McpTransport;
    use axum::extract::State;
    use axum::routing::post;
    use axum::Router;
    use serde_json::Value;
    use std::net::SocketAddr;
    use std::sync::Arc;
    use tokio::net::TcpListener;
    use tokio::sync::Mutex as TokioMutex;

    #[test]
    fn prefixed_name_format() {
        let prefixed = format!("{}__{}", "linear", "search_issues");
        assert_eq!(prefixed, "linear__search_issues");
    }

    #[tokio::test]
    async fn connect_all_nonfatal_on_single_failure() {
        let cfg = McpServerConfig {
            name: "bad".to_string(),
            enabled: true,
            transport: McpTransport::Stdio,
            command: Some("/this/does/not/exist/rikabot".to_string()),
            args: vec![],
            env: HashMap::new(),
            cwd: None,
            url: None,
            headers: HashMap::new(),
            tool_timeout_secs: None,
            init_timeout_secs: None,
        };
        let registry = McpRegistry::connect_all(&[cfg]).await;
        assert!(registry.is_empty());
    }

    #[tokio::test]
    async fn official_style_flow_works_over_stdio() {
        let script = r#"i=0
while IFS= read -r line; do
  i=$((i+1))
  case "$line" in
    *"\"method\":\"initialize\""*)
      echo '{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2024-11-05"}}'
      ;;
    *"\"method\":\"notifications/initialized\""*)
      echo '{"jsonrpc":"2.0","result":{}}'
      ;;
    *"\"method\":\"tools/list\""*)
      echo '{"jsonrpc":"2.0","id":2,"result":{"tools":[{"name":"ping","description":"Ping","inputSchema":{"type":"object","properties":{}}}]}}'
      ;;
    *"\"method\":\"tools/call\""*)
      echo '{"jsonrpc":"2.0","id":3,"result":{"content":[{"type":"text","text":"pong"}]}}'
      ;;
    *)
      echo '{"jsonrpc":"2.0","id":999,"error":{"code":-32601,"message":"unknown"}}'
      ;;
  esac
done"#;

        let cfg = McpServerConfig {
            name: "stdio_mock".to_string(),
            enabled: true,
            transport: McpTransport::Stdio,
            command: Some("sh".to_string()),
            args: vec!["-c".to_string(), script.to_string()],
            env: HashMap::new(),
            cwd: None,
            url: None,
            headers: HashMap::new(),
            tool_timeout_secs: Some(10),
            init_timeout_secs: Some(10),
        };

        let server = McpServer::connect(cfg)
            .await
            .expect("stdio connect should succeed");
        let tools = server.tools().await;
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name, "ping");

        let result = server
            .call_tool("ping", serde_json::json!({}))
            .await
            .expect("tool call should succeed");
        assert_eq!(result["content"][0]["text"], "pong");
    }

    #[derive(Clone, Default)]
    struct HttpMockState {
        methods: Arc<TokioMutex<Vec<String>>>,
    }

    async fn http_mock_handler(
        State(state): State<HttpMockState>,
        axum::Json(body): axum::Json<Value>,
    ) -> axum::Json<Value> {
        let method = body
            .get("method")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        state.methods.lock().await.push(method.clone());

        let id = body.get("id").cloned();
        let response = match method.as_str() {
            "initialize" => serde_json::json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": { "protocolVersion": "2024-11-05" }
            }),
            "notifications/initialized" => serde_json::json!({
                "jsonrpc": "2.0",
                "result": {}
            }),
            "tools/list" => serde_json::json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": {
                    "tools": [
                        {
                            "name": "search",
                            "description": "Search issues",
                            "inputSchema": { "type": "object", "properties": { "query": { "type": "string" } } }
                        }
                    ]
                }
            }),
            "tools/call" => {
                let name = body
                    .get("params")
                    .and_then(|p| p.get("name"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": { "ok": true, "tool": name }
                })
            }
            _ => serde_json::json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": { "code": -32601, "message": "method not found" }
            }),
        };
        axum::Json(response)
    }

    #[tokio::test]
    async fn official_style_flow_works_over_http() {
        let state = HttpMockState::default();
        let app = Router::new()
            .route("/mcp", post(http_mock_handler))
            .with_state(state.clone());
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr: SocketAddr = listener.local_addr().unwrap();
        let server_task = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        let cfg = McpServerConfig {
            name: "http_mock".to_string(),
            enabled: true,
            transport: McpTransport::Http,
            command: None,
            args: vec![],
            env: HashMap::new(),
            cwd: None,
            url: Some(format!("http://{}/mcp", addr)),
            headers: HashMap::new(),
            tool_timeout_secs: Some(10),
            init_timeout_secs: Some(10),
        };

        let server = McpServer::connect(cfg)
            .await
            .expect("http connect should succeed");
        let tools = server.tools().await;
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name, "search");

        let result = server
            .call_tool("search", serde_json::json!({ "query": "abc" }))
            .await
            .expect("http tools/call should succeed");
        assert_eq!(result["ok"], true);
        assert_eq!(result["tool"], "search");

        let methods = state.methods.lock().await.clone();
        assert_eq!(
            methods,
            vec![
                "initialize".to_string(),
                "notifications/initialized".to_string(),
                "tools/list".to_string(),
                "tools/call".to_string()
            ]
        );

        server_task.abort();
    }
}

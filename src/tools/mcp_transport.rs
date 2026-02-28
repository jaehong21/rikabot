use anyhow::{anyhow, bail, Context, Result};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::time::{timeout, Duration};

use crate::config::{McpServerConfig, McpTransport};
use crate::tools::mcp_protocol::{JsonRpcRequest, JsonRpcResponse};

const MAX_LINE_BYTES: usize = 4 * 1024 * 1024;

#[async_trait::async_trait]
pub trait McpTransportConn: Send + Sync {
    async fn send_and_recv(
        &mut self,
        request: &JsonRpcRequest,
        timeout_secs: u64,
    ) -> Result<JsonRpcResponse>;
}

pub struct StdioTransport {
    _child: Child,
    stdin: tokio::process::ChildStdin,
    stdout_lines: tokio::io::Lines<BufReader<tokio::process::ChildStdout>>,
}

impl StdioTransport {
    pub fn new(config: &McpServerConfig) -> Result<Self> {
        let command = config
            .command
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| anyhow!("stdio transport requires command"))?;

        let mut child_cmd = Command::new(command);
        child_cmd
            .args(&config.args)
            .envs(&config.env)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::inherit())
            .kill_on_drop(true);

        if let Some(cwd) = &config.cwd {
            child_cmd.current_dir(cwd);
        }

        let mut child = child_cmd
            .spawn()
            .with_context(|| format!("failed to spawn MCP server `{}`", config.name))?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| anyhow!("no stdin on MCP server `{}`", config.name))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow!("no stdout on MCP server `{}`", config.name))?;

        Ok(Self {
            _child: child,
            stdin,
            stdout_lines: BufReader::new(stdout).lines(),
        })
    }

    async fn send_raw(&mut self, line: &str) -> Result<()> {
        self.stdin
            .write_all(line.as_bytes())
            .await
            .context("failed to write to MCP stdio stdin")?;
        self.stdin
            .write_all(b"\n")
            .await
            .context("failed to write newline to MCP stdio stdin")?;
        self.stdin
            .flush()
            .await
            .context("failed to flush MCP stdio stdin")?;
        Ok(())
    }

    async fn recv_raw(&mut self) -> Result<String> {
        let line = self
            .stdout_lines
            .next_line()
            .await?
            .ok_or_else(|| anyhow!("MCP stdio server closed stdout"))?;

        if line.len() > MAX_LINE_BYTES {
            bail!("MCP stdio response too large: {} bytes", line.len());
        }

        Ok(line)
    }
}

#[async_trait::async_trait]
impl McpTransportConn for StdioTransport {
    async fn send_and_recv(
        &mut self,
        request: &JsonRpcRequest,
        timeout_secs: u64,
    ) -> Result<JsonRpcResponse> {
        let line = serde_json::to_string(request)?;
        self.send_raw(&line).await?;

        let resp_line = timeout(Duration::from_secs(timeout_secs), self.recv_raw())
            .await
            .context("timeout waiting for MCP stdio response")??;

        serde_json::from_str(&resp_line)
            .with_context(|| format!("invalid JSON-RPC response from stdio transport: {resp_line}"))
    }
}

pub struct HttpTransport {
    url: String,
    client: reqwest::Client,
    headers: std::collections::HashMap<String, String>,
    session_id: Option<String>,
}

impl HttpTransport {
    pub fn new(config: &McpServerConfig) -> Result<Self> {
        let url = config
            .url
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| anyhow!("http transport requires url"))?;
        let headers = config.resolved_http_headers()?;
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(config.resolved_tool_timeout_secs()))
            .build()
            .context("failed to build MCP HTTP client")?;

        Ok(Self {
            url: url.to_string(),
            client,
            headers,
            session_id: None,
        })
    }
}

fn parse_sse_jsonrpc(body: &str) -> Result<JsonRpcResponse> {
    for frame in body.split("\n\n") {
        let mut data_lines = Vec::new();
        for line in frame.lines() {
            let trimmed = line.trim_end_matches('\r');
            if let Some(rest) = trimmed.strip_prefix("data:") {
                data_lines.push(rest.trim_start());
            }
        }

        if data_lines.is_empty() {
            continue;
        }

        let data = data_lines.join("\n");
        let payload = data.trim();
        if payload.is_empty() || payload == "[DONE]" {
            continue;
        }

        if let Ok(resp) = serde_json::from_str::<JsonRpcResponse>(payload) {
            return Ok(resp);
        }
    }

    bail!("no JSON-RPC payload found in SSE response");
}

#[async_trait::async_trait]
impl McpTransportConn for HttpTransport {
    async fn send_and_recv(
        &mut self,
        request: &JsonRpcRequest,
        timeout_secs: u64,
    ) -> Result<JsonRpcResponse> {
        let body = serde_json::to_string(request)?;
        let mut req = self
            .client
            .post(&self.url)
            .body(body)
            .header("Content-Type", "application/json")
            // Required by MCP Streamable HTTP transport.
            .header("Accept", "application/json, text/event-stream");

        for (k, v) in &self.headers {
            req = req.header(k, v);
        }
        if let Some(session_id) = &self.session_id {
            req = req.header("MCP-Session-Id", session_id);
        }

        let resp = timeout(Duration::from_secs(timeout_secs), req.send())
            .await
            .context("timeout waiting for MCP HTTP response")?
            .context("HTTP request to MCP server failed")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            if body.is_empty() {
                bail!("MCP server returned HTTP {}", status);
            }
            bail!("MCP server returned HTTP {}: {}", status, body);
        }

        if let Some(session_id) = resp
            .headers()
            .get("MCP-Session-Id")
            .and_then(|v| v.to_str().ok())
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
        {
            self.session_id = Some(session_id);
        }

        let content_type = resp
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_ascii_lowercase();

        let text = resp.text().await.context("failed to read MCP HTTP body")?;
        if text.trim().is_empty() && request.id.is_none() {
            return Ok(JsonRpcResponse {
                jsonrpc: "2.0".to_string(),
                id: None,
                result: Some(serde_json::json!({})),
                error: None,
            });
        }

        if content_type.starts_with("text/event-stream") {
            return parse_sse_jsonrpc(&text).with_context(|| {
                format!("invalid SSE JSON-RPC response from http transport: {text}")
            });
        }

        serde_json::from_str(&text).with_context(|| {
            format!(
                "invalid JSON-RPC response from http transport (content-type: {}): {text}",
                content_type
            )
        })
    }
}

pub fn create_transport(config: &McpServerConfig) -> Result<Box<dyn McpTransportConn>> {
    match config.transport {
        McpTransport::Stdio => Ok(Box::new(StdioTransport::new(config)?)),
        McpTransport::Http => Ok(Box::new(HttpTransport::new(config)?)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::extract::Request;
    use axum::http::{HeaderValue, StatusCode};
    use axum::response::IntoResponse;
    use axum::routing::post;
    use axum::Router;
    use std::collections::HashMap;
    use std::sync::Arc;
    use tokio::net::TcpListener;
    use tokio::sync::Mutex;

    fn base_server(name: &str) -> McpServerConfig {
        McpServerConfig {
            name: name.to_string(),
            enabled: true,
            transport: McpTransport::Stdio,
            command: Some("echo".to_string()),
            args: vec![],
            env: HashMap::new(),
            cwd: None,
            url: None,
            headers: HashMap::new(),
            tool_timeout_secs: None,
            init_timeout_secs: None,
        }
    }

    #[test]
    fn stdio_transport_spawn_failure_is_clean_error() {
        let mut cfg = base_server("bad");
        cfg.command = Some("/this/does/not/exist/rikabot".to_string());
        let err = match StdioTransport::new(&cfg) {
            Ok(_) => panic!("stdio transport creation should fail"),
            Err(e) => e.to_string(),
        };
        assert!(err.contains("failed to spawn"));
    }

    #[test]
    fn http_transport_requires_url() {
        let mut cfg = base_server("http");
        cfg.transport = McpTransport::Http;
        cfg.command = None;
        let err = match HttpTransport::new(&cfg) {
            Ok(_) => panic!("http transport creation should fail"),
            Err(e) => e.to_string(),
        };
        assert!(err.contains("requires url"));
    }

    #[tokio::test]
    async fn http_transport_returns_error_on_non_2xx() {
        let app = Router::new().route(
            "/mcp",
            post(|| async { (axum::http::StatusCode::INTERNAL_SERVER_ERROR, "boom") }),
        );
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        let mut cfg = base_server("http_err");
        cfg.transport = McpTransport::Http;
        cfg.command = None;
        cfg.url = Some(format!("http://{addr}/mcp"));

        let mut t = HttpTransport::new(&cfg).unwrap();
        let req = JsonRpcRequest::new(1, "tools/list", serde_json::json!({}));
        let err = t.send_and_recv(&req, 3).await.unwrap_err().to_string();
        assert!(err.contains("MCP server returned HTTP 500"));

        task.abort();
    }

    #[tokio::test]
    async fn http_transport_sends_required_accept_header() {
        let app = Router::new().route(
            "/mcp",
            post(|req: Request| async move {
                let accept = req
                    .headers()
                    .get(reqwest::header::ACCEPT)
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("");

                if !accept.contains("application/json") || !accept.contains("text/event-stream") {
                    return StatusCode::NOT_ACCEPTABLE.into_response();
                }

                let body = serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": 1,
                    "result": { "ok": true }
                });
                (StatusCode::OK, body.to_string()).into_response()
            }),
        );
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        let mut cfg = base_server("http_accept");
        cfg.transport = McpTransport::Http;
        cfg.command = None;
        cfg.url = Some(format!("http://{addr}/mcp"));

        let mut t = HttpTransport::new(&cfg).unwrap();
        let req = JsonRpcRequest::new(1, "tools/list", serde_json::json!({}));
        let resp = t.send_and_recv(&req, 3).await.unwrap();
        assert_eq!(resp.result.unwrap()["ok"], true);

        task.abort();
    }

    #[tokio::test]
    async fn http_transport_reuses_mcp_session_id() {
        let seen_session = Arc::new(Mutex::new(Vec::<String>::new()));
        let seen_session_cloned = seen_session.clone();

        let app = Router::new().route(
            "/mcp",
            post(move |req: Request| {
                let seen_session_cloned = seen_session_cloned.clone();
                async move {
                    let session = req
                        .headers()
                        .get("MCP-Session-Id")
                        .and_then(|v| v.to_str().ok())
                        .unwrap_or("")
                        .to_string();
                    seen_session_cloned.lock().await.push(session.clone());

                    let body = serde_json::json!({
                        "jsonrpc": "2.0",
                        "id": 1,
                        "result": { "ok": true }
                    });
                    let mut resp =
                        axum::response::Response::new(axum::body::Body::from(body.to_string()));
                    if session.is_empty() {
                        resp.headers_mut()
                            .insert("MCP-Session-Id", HeaderValue::from_static("sess-123"));
                    }
                    resp
                }
            }),
        );

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        let mut cfg = base_server("http_session");
        cfg.transport = McpTransport::Http;
        cfg.command = None;
        cfg.url = Some(format!("http://{addr}/mcp"));

        let mut t = HttpTransport::new(&cfg).unwrap();
        let req = JsonRpcRequest::new(1, "initialize", serde_json::json!({}));
        let _ = t.send_and_recv(&req, 3).await.unwrap();
        let req2 = JsonRpcRequest::new(1, "tools/list", serde_json::json!({}));
        let _ = t.send_and_recv(&req2, 3).await.unwrap();

        let seen = seen_session.lock().await.clone();
        assert_eq!(seen.len(), 2);
        assert_eq!(seen[0], "");
        assert_eq!(seen[1], "sess-123");

        task.abort();
    }

    #[tokio::test]
    async fn http_transport_parses_sse_jsonrpc_response() {
        let app = Router::new().route(
            "/mcp",
            post(|| async move {
                let sse = "event: message\ndata: {\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{\"ok\":true}}\n\n";
                (
                    StatusCode::OK,
                    [(reqwest::header::CONTENT_TYPE.as_str(), "text/event-stream")],
                    sse,
                )
            }),
        );
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        let mut cfg = base_server("http_sse");
        cfg.transport = McpTransport::Http;
        cfg.command = None;
        cfg.url = Some(format!("http://{addr}/mcp"));

        let mut t = HttpTransport::new(&cfg).unwrap();
        let req = JsonRpcRequest::new(1, "tools/list", serde_json::json!({}));
        let resp = t.send_and_recv(&req, 3).await.unwrap();
        assert_eq!(resp.result.unwrap()["ok"], true);

        task.abort();
    }
}

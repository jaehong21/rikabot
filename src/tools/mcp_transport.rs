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
        })
    }
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
            .header("Content-Type", "application/json");

        for (k, v) in &self.headers {
            req = req.header(k, v);
        }

        let resp = timeout(Duration::from_secs(timeout_secs), req.send())
            .await
            .context("timeout waiting for MCP HTTP response")?
            .context("HTTP request to MCP server failed")?;

        if !resp.status().is_success() {
            bail!("MCP server returned HTTP {}", resp.status());
        }

        let text = resp.text().await.context("failed to read MCP HTTP body")?;
        serde_json::from_str(&text)
            .with_context(|| format!("invalid JSON-RPC response from http transport: {text}"))
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
    use axum::routing::post;
    use axum::Router;
    use std::collections::HashMap;
    use tokio::net::TcpListener;

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
}

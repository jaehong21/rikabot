use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::Arc;
use std::sync::RwLock;
use tokio::sync::RwLock as TokioRwLock;

use crate::permissions::PermissionEngine;

pub mod filesystem_glob;
pub mod filesystem_read;
pub mod filesystem_search;
pub mod filesystem_write;
pub mod mcp_client;
pub mod mcp_oauth;
pub mod mcp_protocol;
pub mod mcp_tool;
pub mod mcp_transport;
pub mod shell;
pub mod web_fetch;
pub mod web_search;

// Re-export ToolSpec from providers (single source of truth).
pub use crate::providers::ToolSpec;

// ── Core types ──────────────────────────────────────────────────────────────

/// Result of a tool execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    pub success: bool,
    pub output: String,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ToolCallStatus {
    Success,
    Failed,
    Denied,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolExecutionResult {
    pub status: ToolCallStatus,
    pub output: String,
    pub error: Option<String>,
}

impl ToolExecutionResult {
    fn success(output: String) -> Self {
        Self {
            status: ToolCallStatus::Success,
            output,
            error: None,
        }
    }

    fn failed(output: String, error: Option<String>) -> Self {
        Self {
            status: ToolCallStatus::Failed,
            output,
            error,
        }
    }

    fn denied(reason: String) -> Self {
        Self {
            status: ToolCallStatus::Denied,
            output: reason.clone(),
            error: Some(reason),
        }
    }
}

// ── Tool trait ──────────────────────────────────────────────────────────────

/// Core tool trait -- implement for any capability.
#[async_trait]
pub trait Tool: Send + Sync {
    /// Tool name (used in LLM function calling).
    fn name(&self) -> &str;

    /// Human-readable description.
    fn description(&self) -> &str;

    /// JSON schema for parameters.
    fn parameters_schema(&self) -> serde_json::Value;

    /// Execute the tool with given arguments.
    async fn execute(&self, args: serde_json::Value) -> Result<ToolResult>;

    /// Get the full spec for LLM registration.
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: self.name().to_string(),
            description: self.description().to_string(),
            parameters: self.parameters_schema(),
        }
    }
}

// ── Tool registry ───────────────────────────────────────────────────────────

/// Registry holding all available tools.
#[derive(Clone)]
pub struct ToolRegistry {
    tools: Arc<RwLock<Vec<Arc<dyn Tool>>>>,
    permission_engine: Arc<TokioRwLock<PermissionEngine>>,
}

impl ToolRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self::with_permission_engine(Arc::new(TokioRwLock::new(
            PermissionEngine::disabled_allow_all(),
        )))
    }

    pub fn with_permission_engine(permission_engine: Arc<TokioRwLock<PermissionEngine>>) -> Self {
        Self {
            tools: Arc::new(RwLock::new(Vec::new())),
            permission_engine,
        }
    }

    /// Register a new tool.
    pub fn register(&mut self, tool: Box<dyn Tool>) {
        if let Ok(mut guard) = self.tools.write() {
            guard.push(Arc::from(tool));
        } else {
            tracing::error!("tool registry write lock poisoned while registering tool");
        }
    }

    pub async fn register_mcp_tools(
        &mut self,
        registry: Arc<mcp_client::McpRegistry>,
    ) -> Result<usize> {
        let mut added = 0usize;
        let mut names = registry.tool_names();
        names.sort();
        for prefixed in names {
            let Some(def) = registry.get_tool_def(&prefixed).await else {
                continue;
            };
            self.register(Box::new(mcp_tool::McpToolWrapper::new(
                prefixed,
                def,
                registry.clone(),
            )));
            added += 1;
        }
        Ok(added)
    }

    /// Get specs for all registered tools (for sending to the LLM).
    pub fn specs(&self) -> Vec<ToolSpec> {
        match self.tools.read() {
            Ok(tools) => tools.iter().map(|t| t.spec()).collect(),
            Err(_) => {
                tracing::error!("tool registry read lock poisoned while collecting specs");
                Vec::new()
            }
        }
    }

    /// Execute a tool by name with the given arguments.
    pub async fn execute(
        &self,
        name: &str,
        args: serde_json::Value,
    ) -> Result<ToolExecutionResult> {
        self.execute_internal(name, args, true).await
    }

    /// Execute a tool call bypassing permission checks (for explicit user approvals).
    pub async fn execute_without_permissions(
        &self,
        name: &str,
        args: serde_json::Value,
    ) -> Result<ToolExecutionResult> {
        self.execute_internal(name, args, false).await
    }

    async fn execute_internal(
        &self,
        name: &str,
        args: serde_json::Value,
        enforce_permissions: bool,
    ) -> Result<ToolExecutionResult> {
        let tool = {
            let tools = self
                .tools
                .read()
                .map_err(|_| anyhow::anyhow!("tool registry read lock poisoned"))?;
            tools.iter().find(|t| t.name() == name).cloned()
        }
        .ok_or_else(|| anyhow::anyhow!("Unknown tool: {}", name))?;

        if enforce_permissions {
            let permission_args = enrich_args_for_permissions(name, &args);
            let decision = {
                let permissions = self.permission_engine.read().await;
                permissions.evaluate(name, &permission_args)
            };
            if !decision.allowed {
                let args_preview = serde_json::to_string(&args)
                    .unwrap_or_else(|_| "<unserializable args>".to_string());
                let args_preview = if args_preview.chars().count() > 512 {
                    format!("{}...", args_preview.chars().take(512).collect::<String>())
                } else {
                    args_preview
                };
                tracing::warn!(
                    tool = %name,
                    args = %args_preview,
                    reason = %decision.reason,
                    "Tool call denied by permissions"
                );
                return Ok(ToolExecutionResult::denied(decision.reason));
            }
        }

        let result = tool.execute(args).await?;
        if result.success {
            Ok(ToolExecutionResult::success(result.output))
        } else {
            let output = if result.output.trim().is_empty() {
                result.error.clone().unwrap_or_default()
            } else {
                result.output
            };
            Ok(ToolExecutionResult::failed(output, result.error))
        }
    }
}

// ── Default registry ────────────────────────────────────────────────────────

/// Create a ToolRegistry pre-loaded with the default tools (shell).
pub fn default_registry(
    workspace_dir: &Path,
    permission_engine: Arc<TokioRwLock<PermissionEngine>>,
    web_fetch_cfg: &crate::config::WebFetchConfig,
    web_search_cfg: &crate::config::WebSearchConfig,
) -> ToolRegistry {
    let mut registry = ToolRegistry::with_permission_engine(permission_engine);
    registry.register(Box::new(shell::ShellTool::with_workspace_dir(
        30,
        workspace_dir.to_path_buf(),
    ))); // 30 second timeout
    registry.register(Box::new(
        filesystem_read::FilesystemReadTool::with_workspace_dir(workspace_dir.to_path_buf()),
    ));
    registry.register(Box::new(
        filesystem_write::FilesystemWriteTool::with_workspace_dir(workspace_dir.to_path_buf()),
    ));
    registry.register(Box::new(
        filesystem_glob::FilesystemGlobTool::with_workspace_dir(workspace_dir.to_path_buf()),
    ));
    registry.register(Box::new(
        filesystem_search::FilesystemSearchTool::with_workspace_dir(workspace_dir.to_path_buf()),
    ));
    if web_fetch_cfg.enabled {
        registry.register(Box::new(web_fetch::WebFetchTool::new(
            web_fetch_cfg.clone(),
        )));
    }
    if web_search_cfg.enabled {
        registry.register(Box::new(web_search::WebSearchTool::new(
            web_search_cfg.clone(),
        )));
    }
    registry
}

fn enrich_args_for_permissions(name: &str, args: &serde_json::Value) -> serde_json::Value {
    if !name.eq_ignore_ascii_case("web_fetch") {
        return args.clone();
    }

    let mut enriched = args.clone();
    let url = args
        .get("url")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    if url.is_empty() {
        return enriched;
    }

    let domain = url::Url::parse(url)
        .ok()
        .and_then(|parsed| parsed.host_str().map(|host| host.to_ascii_lowercase()));

    if let (Some(domain), serde_json::Value::Object(map)) = (domain, &mut enriched) {
        map.insert("domain".to_string(), serde_json::Value::String(domain));
    }

    enriched
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{
        PermissionsConfig, ToolPermissionsConfig, WebFetchConfig, WebSearchConfig,
    };

    #[test]
    fn enriches_web_fetch_args_with_domain() {
        let args = serde_json::json!({
            "url": "https://Docs.OpenClaw.ai/path/to/page?x=1"
        });
        let enriched = enrich_args_for_permissions("web_fetch", &args);
        assert_eq!(
            enriched.get("domain").and_then(serde_json::Value::as_str),
            Some("docs.openclaw.ai")
        );
    }

    #[test]
    fn leaves_non_web_fetch_args_untouched() {
        let args = serde_json::json!({
            "query": "rust tooling"
        });
        let enriched = enrich_args_for_permissions("web_search", &args);
        assert_eq!(enriched, args);
    }

    #[tokio::test]
    async fn permission_engine_matches_domain_selector_for_web_fetch() {
        let permission_cfg = PermissionsConfig {
            enabled: true,
            tools: ToolPermissionsConfig {
                allow: vec!["web_fetch(domain:docs.openclaw.ai)".to_string()],
                deny: vec![],
            },
        };
        let engine = PermissionEngine::from_config(&permission_cfg).unwrap();

        let allowed = engine.evaluate(
            "web_fetch",
            &enrich_args_for_permissions(
                "web_fetch",
                &serde_json::json!({"url":"https://docs.openclaw.ai/docs"}),
            ),
        );
        let denied = engine.evaluate(
            "web_fetch",
            &enrich_args_for_permissions(
                "web_fetch",
                &serde_json::json!({"url":"https://github.com/openclaw"}),
            ),
        );

        assert!(allowed.allowed);
        assert!(!denied.allowed);
    }

    #[tokio::test]
    async fn default_registry_registers_web_tools_only_when_enabled() {
        let permission_engine = Arc::new(TokioRwLock::new(PermissionEngine::disabled_allow_all()));
        let workspace = std::env::temp_dir();

        let disabled = default_registry(
            &workspace,
            permission_engine.clone(),
            &WebFetchConfig {
                enabled: false,
                ..WebFetchConfig::default()
            },
            &WebSearchConfig {
                enabled: false,
                ..WebSearchConfig::default()
            },
        );
        let disabled_specs = disabled.specs();
        assert!(!disabled_specs.iter().any(|s| s.name == "web_fetch"));
        assert!(!disabled_specs.iter().any(|s| s.name == "web_search"));

        let enabled = default_registry(
            &workspace,
            permission_engine,
            &WebFetchConfig {
                enabled: true,
                ..WebFetchConfig::default()
            },
            &WebSearchConfig {
                enabled: true,
                ..WebSearchConfig::default()
            },
        );
        let enabled_specs = enabled.specs();
        assert!(enabled_specs.iter().any(|s| s.name == "web_fetch"));
        assert!(enabled_specs.iter().any(|s| s.name == "web_search"));
    }

    #[tokio::test]
    async fn failed_tool_uses_error_text_as_output_when_output_empty() {
        let mut registry = ToolRegistry::new();
        registry.register(Box::new(FailingTool));

        let result = registry
            .execute("failing_tool", serde_json::json!({}))
            .await
            .unwrap();

        assert_eq!(result.status, ToolCallStatus::Failed);
        assert_eq!(result.output, "boom");
        assert_eq!(result.error.as_deref(), Some("boom"));
    }
}
struct FailingTool;

#[async_trait]
impl Tool for FailingTool {
    fn name(&self) -> &str {
        "failing_tool"
    }

    fn description(&self) -> &str {
        "fails with empty output"
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {}
        })
    }

    async fn execute(&self, _args: serde_json::Value) -> Result<ToolResult> {
        Ok(ToolResult {
            success: false,
            output: String::new(),
            error: Some("boom".to_string()),
        })
    }
}

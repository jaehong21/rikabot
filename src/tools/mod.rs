use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::Arc;
use tokio::sync::RwLock;

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
pub struct ToolRegistry {
    tools: Vec<Box<dyn Tool>>,
    permission_engine: Arc<RwLock<PermissionEngine>>,
}

impl ToolRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self::with_permission_engine(Arc::new(
            RwLock::new(PermissionEngine::disabled_allow_all()),
        ))
    }

    pub fn with_permission_engine(permission_engine: Arc<RwLock<PermissionEngine>>) -> Self {
        Self {
            tools: Vec::new(),
            permission_engine,
        }
    }

    /// Register a new tool.
    pub fn register(&mut self, tool: Box<dyn Tool>) {
        self.tools.push(tool);
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
        self.tools.iter().map(|t| t.spec()).collect()
    }

    /// Execute a tool by name with the given arguments.
    pub async fn execute(&self, name: &str, args: serde_json::Value) -> Result<ToolResult> {
        let tool = self
            .tools
            .iter()
            .find(|t| t.name() == name)
            .ok_or_else(|| anyhow::anyhow!("Unknown tool: {}", name))?;

        let decision = {
            let permissions = self.permission_engine.read().await;
            permissions.evaluate(name, &args)
        };
        if !decision.allowed {
            return Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some(decision.reason),
            });
        }

        tool.execute(args).await
    }
}

// ── Default registry ────────────────────────────────────────────────────────

/// Create a ToolRegistry pre-loaded with the default tools (shell).
pub fn default_registry(
    workspace_dir: &Path,
    permission_engine: Arc<RwLock<PermissionEngine>>,
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
    registry
}

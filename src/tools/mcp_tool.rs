use std::sync::Arc;

use async_trait::async_trait;

use crate::tools::mcp_client::McpRegistry;
use crate::tools::mcp_protocol::McpToolDef;
use crate::tools::{Tool, ToolResult};

pub struct McpToolWrapper {
    prefixed_name: String,
    description: String,
    input_schema: serde_json::Value,
    registry: Arc<McpRegistry>,
}

impl McpToolWrapper {
    pub fn new(prefixed_name: String, def: McpToolDef, registry: Arc<McpRegistry>) -> Self {
        Self {
            prefixed_name,
            description: def.description.unwrap_or_else(|| "MCP tool".to_string()),
            input_schema: def.input_schema,
            registry,
        }
    }
}

#[async_trait]
impl Tool for McpToolWrapper {
    fn name(&self) -> &str {
        &self.prefixed_name
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn parameters_schema(&self) -> serde_json::Value {
        self.input_schema.clone()
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        match self.registry.call_tool(&self.prefixed_name, args).await {
            Ok(output) => Ok(ToolResult {
                success: true,
                output,
                error: None,
            }),
            Err(e) => Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some(e.to_string()),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[tokio::test]
    async fn wrapper_forwards_name_description_and_schema() {
        let registry = Arc::new(crate::tools::mcp_client::McpRegistry::connect_all(&[]).await);
        let def = McpToolDef {
            name: "search".to_string(),
            description: Some("Search issues".to_string()),
            input_schema: serde_json::json!({"type":"object","properties":{"query":{"type":"string"}}}),
        };
        let wrapper = McpToolWrapper::new("linear__search".to_string(), def, registry);

        assert_eq!(wrapper.name(), "linear__search");
        assert_eq!(wrapper.description(), "Search issues");
        assert_eq!(wrapper.parameters_schema()["type"], "object");
    }

    #[tokio::test]
    async fn wrapper_returns_failure_for_unknown_tool() {
        let cfg = crate::config::McpServerConfig {
            name: "disabled".to_string(),
            enabled: false,
            transport: crate::config::McpTransport::Stdio,
            command: Some("echo".to_string()),
            args: vec![],
            env: HashMap::new(),
            cwd: None,
            url: None,
            headers: HashMap::new(),
            tool_timeout_secs: None,
            init_timeout_secs: None,
        };
        let registry = Arc::new(crate::tools::mcp_client::McpRegistry::connect_all(&[cfg]).await);
        let def = McpToolDef {
            name: "search".to_string(),
            description: None,
            input_schema: serde_json::json!({"type":"object"}),
        };
        let wrapper = McpToolWrapper::new("linear__search".to_string(), def, registry);
        let result = wrapper
            .execute(serde_json::json!({"query":"x"}))
            .await
            .unwrap();
        assert!(!result.success);
        assert!(result
            .error
            .as_deref()
            .unwrap_or("")
            .contains("unknown MCP tool"));
    }
}

use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

pub mod openai_codex;
pub mod openrouter;

/// A single message in a conversation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

impl ChatMessage {
    pub fn system(content: &str) -> Self {
        Self {
            role: "system".into(),
            content: content.into(),
        }
    }
    pub fn user(content: &str) -> Self {
        Self {
            role: "user".into(),
            content: content.into(),
        }
    }
    pub fn assistant(content: &str) -> Self {
        Self {
            role: "assistant".into(),
            content: content.into(),
        }
    }
    pub fn tool(content: &str) -> Self {
        Self {
            role: "tool".into(),
            content: content.into(),
        }
    }
}

/// A tool call requested by the LLM.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: String, // JSON string
}

/// Specification for a tool that can be offered to the LLM.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

/// Raw token counts from a single LLM API response.
#[derive(Debug, Clone, Default)]
pub struct TokenUsage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: u32,
}

/// An LLM response that may contain text, tool calls, or both.
#[derive(Debug, Clone)]
pub struct ChatResponse {
    /// Text content of the response (may be empty if only tool calls).
    pub text: Option<String>,
    /// Tool calls requested by the LLM.
    pub tool_calls: Vec<ToolCall>,
    /// Token usage reported by the provider, if available.
    pub usage: Option<TokenUsage>,
}

impl ChatResponse {
    /// True when the LLM wants to invoke at least one tool.
    pub fn has_tool_calls(&self) -> bool {
        !self.tool_calls.is_empty()
    }

    /// Convenience: return text content or empty string.
    pub fn text_or_empty(&self) -> &str {
        self.text.as_deref().unwrap_or("")
    }
}

#[async_trait]
pub trait Provider: Send + Sync {
    /// Whether this provider supports native tool calling via API primitives.
    fn supports_native_tools(&self) -> bool;

    /// Send a chat completion request with optional tool definitions.
    async fn chat(
        &self,
        messages: &[ChatMessage],
        tools: Option<&[ToolSpec]>,
        model: &str,
        temperature: f64,
    ) -> Result<ChatResponse>;
}

/// Create a provider instance from configuration.
pub fn create_provider(config: &crate::config::AppConfig) -> Result<Box<dyn Provider>> {
    match config.provider.as_str() {
        "openrouter" => {
            let cfg = config.providers.openrouter.as_ref().ok_or_else(|| {
                anyhow::anyhow!(
                    "provider is 'openrouter' but [providers.openrouter] not configured"
                )
            })?;
            let api_key = cfg.resolve_api_key()?;
            Ok(Box::new(openrouter::OpenRouterProvider::new(&api_key)))
        }
        "openai_codex" => {
            let cfg = config.providers.openai_codex.as_ref().ok_or_else(|| {
                anyhow::anyhow!(
                    "provider is 'openai_codex' but [providers.openai_codex] not configured"
                )
            })?;
            let oauth_token = cfg.resolve_oauth_token()?;
            let account_id = cfg.resolve_account_id()?;
            Ok(Box::new(openai_codex::OpenAiCodexProvider::new(
                &oauth_token,
                &account_id,
            )))
        }
        other => anyhow::bail!("Unknown provider: {}", other),
    }
}

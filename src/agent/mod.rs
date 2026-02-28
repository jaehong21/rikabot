use anyhow::Result;
use serde::Serialize;
use std::time::Instant;
use tokio::sync::mpsc;

use crate::providers::{ChatMessage, ChatResponse, Provider, TokenUsage, ToolSpec};
use crate::tools::ToolRegistry;

// ── Constants ────────────────────────────────────────────────────────────────

/// Maximum number of tool-call iterations before the agent stops.
const MAX_ITERATIONS: usize = 20;

// ── Agent events (sent via channel to WebSocket / consumers) ─────────────────

/// Events emitted by the agent loop, forwarded to WebSocket clients.
///
/// Sent through an `mpsc::unbounded_channel<AgentEvent>`.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentEvent {
    /// Text chunk from assistant alongside tool calls.
    Chunk { content: String },
    /// A tool call is starting.
    ToolCallStart {
        name: String,
        args: serde_json::Value,
    },
    /// A tool call has finished.
    ToolCallResult {
        name: String,
        output: String,
        success: bool,
    },
    /// Final answer from the assistant.
    Done {
        full_response: String,
        elapsed_ms: u64,
        tool_call_count: u32,
        tool_call_success: u32,
        tool_call_failed: u32,
        usage: Option<TokenUsage>,
    },
    /// An error occurred.
    Error { message: String },
}

// ── Agent ────────────────────────────────────────────────────────────────────

/// The core agent that orchestrates the iterative LLM + tool execution loop.
///
/// Design (from PRD):
///   1. Build messages: `[system_prompt, ...history, user_message]`
///   2. Loop (max [`MAX_ITERATIONS`] iterations):
///      a. `response = provider.chat(messages, tools, model, temperature)`
///      b. If response has tool calls:
///         - Append assistant message (tool calls encoded as JSON in content)
///         - For each tool call: execute, append tool result message, send [`AgentEvent`]
///      c. Else: send [`AgentEvent::Done`] with final text, break
pub struct Agent {
    provider: Box<dyn Provider>,
    tool_registry: ToolRegistry,
    model: String,
    temperature: f64,
}

impl Agent {
    /// Create a new Agent.
    pub fn new(
        provider: Box<dyn Provider>,
        tool_registry: ToolRegistry,
        model: String,
        temperature: f64,
    ) -> Self {
        Self {
            provider,
            tool_registry,
            model,
            temperature,
        }
    }

    /// Run the agent loop for a single user message within an ongoing conversation.
    ///
    /// `history` is the mutable conversation history (caller owns it).
    /// Events are sent through `event_tx` for the WebSocket layer to forward.
    ///
    /// The loop follows the zeroclaw/nanobot pattern:
    ///   - Assistant messages with tool calls encode them as JSON in the content field
    ///   - Tool result messages encode `tool_call_id` + `content` as JSON
    ///   - The loop continues until the LLM responds without tool calls, or [`MAX_ITERATIONS`]
    pub async fn run(
        &self,
        system_prompt: String,
        history: &mut Vec<ChatMessage>,
        user_message: String,
        event_tx: mpsc::UnboundedSender<AgentEvent>,
    ) -> Result<()> {
        // Append user message to history
        history.push(ChatMessage::user(&user_message));

        // Gather tool specs for LLM.
        // Convert from tools::ToolSpec to providers::ToolSpec (same shape, separate types).
        let tool_specs: Vec<ToolSpec> = self
            .tool_registry
            .specs()
            .into_iter()
            .map(|ts| ToolSpec {
                name: ts.name,
                description: ts.description,
                parameters: ts.parameters,
            })
            .collect();

        // Run the iterative loop
        self.run_loop(&system_prompt, history, &tool_specs, &event_tx)
            .await
    }

    /// The inner agent loop: call LLM, handle tool calls, repeat.
    async fn run_loop(
        &self,
        system_prompt: &str,
        history: &mut Vec<ChatMessage>,
        tool_specs: &[ToolSpec],
        tx: &mpsc::UnboundedSender<AgentEvent>,
    ) -> Result<()> {
        let started_at = Instant::now();
        let mut usage_total = TokenUsage::default();
        let mut has_usage = false;
        let mut tool_call_count: u32 = 0;
        let mut tool_call_success: u32 = 0;
        let mut tool_call_failed: u32 = 0;

        for iteration in 0..MAX_ITERATIONS {
            tracing::debug!("Agent loop iteration {}", iteration);

            // Build full message list: system prompt + conversation history
            let mut messages = Vec::with_capacity(1 + history.len());
            messages.push(ChatMessage::system(system_prompt));
            messages.extend(history.iter().cloned());

            // Call LLM
            let response: ChatResponse = match self
                .provider
                .chat(&messages, Some(tool_specs), &self.model, self.temperature)
                .await
            {
                Ok(r) => r,
                Err(e) => {
                    let _ = tx.send(AgentEvent::Error {
                        message: e.to_string(),
                    });
                    return Err(e);
                }
            };

            if let Some(usage) = &response.usage {
                has_usage = true;
                usage_total.prompt_tokens = usage_total
                    .prompt_tokens
                    .saturating_add(usage.prompt_tokens);
                usage_total.completion_tokens = usage_total
                    .completion_tokens
                    .saturating_add(usage.completion_tokens);
                usage_total.total_tokens =
                    usage_total.total_tokens.saturating_add(usage.total_tokens);
            }

            // ── No tool calls => final response ─────────────────────────
            if response.tool_calls.is_empty() {
                let final_text = response.text.unwrap_or_else(|| "(no response)".to_string());

                history.push(ChatMessage::assistant(&final_text));

                let elapsed_ms =
                    u64::try_from(started_at.elapsed().as_millis()).unwrap_or(u64::MAX);
                let _ = tx.send(AgentEvent::Done {
                    full_response: final_text,
                    elapsed_ms,
                    tool_call_count,
                    tool_call_success,
                    tool_call_failed,
                    usage: has_usage.then_some(usage_total.clone()),
                });
                return Ok(());
            }

            // ── Has tool calls => process them ──────────────────────────

            // If there is text alongside tool calls, send it as a chunk
            if let Some(ref text) = response.text {
                if !text.is_empty() {
                    let _ = tx.send(AgentEvent::Chunk {
                        content: text.clone(),
                    });
                }
            }

            // Encode tool calls into assistant message content (zeroclaw pattern).
            // The assistant message stores structured JSON so the conversation
            // history can be replayed correctly on subsequent LLM calls.
            let tool_calls_json: Vec<serde_json::Value> = response
                .tool_calls
                .iter()
                .map(|tc| {
                    serde_json::json!({
                        "id": tc.id,
                        "name": tc.name,
                        "arguments": tc.arguments,
                    })
                })
                .collect();

            let assistant_content = serde_json::json!({
                "tool_calls": tool_calls_json,
                "content": response.text.as_deref().unwrap_or(""),
            })
            .to_string();

            history.push(ChatMessage::assistant(&assistant_content));

            // Execute each tool call sequentially
            for tc in &response.tool_calls {
                tool_call_count = tool_call_count.saturating_add(1);

                let args: serde_json::Value = serde_json::from_str(&tc.arguments)
                    .unwrap_or(serde_json::Value::Object(serde_json::Map::new()));

                let _ = tx.send(AgentEvent::ToolCallStart {
                    name: tc.name.clone(),
                    args: args.clone(),
                });

                let result = self.tool_registry.execute(&tc.name, args).await;

                match result {
                    Ok(tool_result) => {
                        if tool_result.success {
                            tool_call_success = tool_call_success.saturating_add(1);
                        } else {
                            tool_call_failed = tool_call_failed.saturating_add(1);
                        }

                        let _ = tx.send(AgentEvent::ToolCallResult {
                            name: tc.name.clone(),
                            output: tool_result.output.clone(),
                            success: tool_result.success,
                        });

                        // Encode tool result as JSON in tool message (zeroclaw pattern)
                        let tool_msg_content = serde_json::json!({
                            "tool_call_id": tc.id,
                            "content": tool_result.output,
                        })
                        .to_string();

                        history.push(ChatMessage::tool(&tool_msg_content));
                    }
                    Err(e) => {
                        let error_output = format!("Tool execution error: {}", e);
                        tool_call_failed = tool_call_failed.saturating_add(1);

                        let _ = tx.send(AgentEvent::ToolCallResult {
                            name: tc.name.clone(),
                            output: error_output.clone(),
                            success: false,
                        });

                        let tool_msg_content = serde_json::json!({
                            "tool_call_id": tc.id,
                            "content": error_output,
                        })
                        .to_string();

                        history.push(ChatMessage::tool(&tool_msg_content));
                    }
                }
            }

            // Continue loop -- LLM will see tool results and decide next action
        }

        // Hit max iterations without a final response
        let msg = format!(
            "Agent reached maximum iterations ({}) without a final response.",
            MAX_ITERATIONS
        );
        let _ = tx.send(AgentEvent::Error {
            message: msg.clone(),
        });
        anyhow::bail!(msg)
    }
}

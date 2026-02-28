use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::providers::{ChatMessage, ChatResponse, Provider, TokenUsage, ToolSpec};
use crate::tools::{ToolCallStatus, ToolRegistry};

// ── Constants ────────────────────────────────────────────────────────────────

/// Maximum number of tool-call iterations before the agent stops.
const MAX_ITERATIONS: usize = 30;
const TOOL_APPROVAL_TIMEOUT: Duration = Duration::from_secs(180);

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ToolApprovalDecisionKind {
    AllowPersist,
    AllowOnce,
    Deny,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ToolApprovalDecision {
    pub request_id: String,
    pub decision: ToolApprovalDecisionKind,
}

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
        call_id: String,
        name: String,
        args: serde_json::Value,
    },
    /// A tool call has finished.
    ToolCallResult {
        call_id: String,
        name: String,
        output: String,
        success: bool,
        status: ToolCallStatus,
        approval_request_id: Option<String>,
        awaiting_approval: bool,
    },
    ToolApprovalRequired {
        request_id: String,
        call_id: String,
        name: String,
        args: serde_json::Value,
        deny_reason: String,
        suggested_allow_rule: String,
    },
    /// Final answer from the assistant.
    Done {
        full_response: String,
        elapsed_ms: u64,
        tool_call_count: u32,
        tool_call_success: u32,
        tool_call_failed: u32,
        tool_call_denied: u32,
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
        mut approval_rx: mpsc::UnboundedReceiver<ToolApprovalDecision>,
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
        self.run_loop(
            &system_prompt,
            history,
            &tool_specs,
            &event_tx,
            &mut approval_rx,
        )
        .await
    }

    /// The inner agent loop: call LLM, handle tool calls, repeat.
    async fn run_loop(
        &self,
        system_prompt: &str,
        history: &mut Vec<ChatMessage>,
        tool_specs: &[ToolSpec],
        tx: &mpsc::UnboundedSender<AgentEvent>,
        approval_rx: &mut mpsc::UnboundedReceiver<ToolApprovalDecision>,
    ) -> Result<()> {
        let started_at = Instant::now();
        let mut usage_total = TokenUsage::default();
        let mut has_usage = false;
        let mut tool_call_count: u32 = 0;
        let mut tool_call_success: u32 = 0;
        let mut tool_call_failed: u32 = 0;
        let mut tool_call_denied: u32 = 0;

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
                    tool_call_denied,
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
                    call_id: tc.id.clone(),
                    name: tc.name.clone(),
                    args: args.clone(),
                });

                let initial_result = self.tool_registry.execute(&tc.name, args.clone()).await;

                match initial_result {
                    Ok(tool_result) => {
                        if tool_result.status == ToolCallStatus::Denied {
                            let request_id = Uuid::new_v4().to_string();
                            let deny_reason = tool_result
                                .error
                                .clone()
                                .unwrap_or_else(|| tool_result.output.clone());

                            self.emit_tool_result(
                                tx,
                                &tc.id,
                                &tc.name,
                                deny_reason.clone(),
                                ToolCallStatus::Denied,
                                Some(request_id.clone()),
                                true,
                            );

                            let _ = tx.send(AgentEvent::ToolApprovalRequired {
                                request_id: request_id.clone(),
                                call_id: tc.id.clone(),
                                name: tc.name.clone(),
                                args: args.clone(),
                                deny_reason,
                                suggested_allow_rule: suggested_allow_rule(&tc.name, &args),
                            });

                            let decision = self
                                .await_tool_approval_decision(approval_rx, &request_id)
                                .await;

                            let (final_status, final_output) = match decision {
                                Some(ToolApprovalDecisionKind::AllowPersist) => {
                                    match self.tool_registry.execute(&tc.name, args.clone()).await {
                                        Ok(result) => (result.status, result.output),
                                        Err(err) => (
                                            ToolCallStatus::Failed,
                                            format!("Tool execution error: {}", err),
                                        ),
                                    }
                                }
                                Some(ToolApprovalDecisionKind::AllowOnce) => {
                                    match self
                                        .tool_registry
                                        .execute_without_permissions(&tc.name, args.clone())
                                        .await
                                    {
                                        Ok(result) => (result.status, result.output),
                                        Err(err) => (
                                            ToolCallStatus::Failed,
                                            format!("Tool execution error: {}", err),
                                        ),
                                    }
                                }
                                Some(ToolApprovalDecisionKind::Deny) => (
                                    ToolCallStatus::Denied,
                                    "Tool call denied by user decision.".to_string(),
                                ),
                                None => (
                                    ToolCallStatus::Denied,
                                    "Tool approval timed out; tool call denied.".to_string(),
                                ),
                            };

                            match final_status {
                                ToolCallStatus::Success => {
                                    tool_call_success = tool_call_success.saturating_add(1)
                                }
                                ToolCallStatus::Failed => {
                                    tool_call_failed = tool_call_failed.saturating_add(1)
                                }
                                ToolCallStatus::Denied => {
                                    tool_call_denied = tool_call_denied.saturating_add(1)
                                }
                            }

                            self.emit_tool_result(
                                tx,
                                &tc.id,
                                &tc.name,
                                final_output.clone(),
                                final_status.clone(),
                                Some(request_id.clone()),
                                false,
                            );

                            self.push_tool_history(history, &tc.id, final_output, final_status);
                        } else {
                            match tool_result.status {
                                ToolCallStatus::Success => {
                                    tool_call_success = tool_call_success.saturating_add(1)
                                }
                                ToolCallStatus::Failed => {
                                    tool_call_failed = tool_call_failed.saturating_add(1)
                                }
                                ToolCallStatus::Denied => {
                                    tool_call_denied = tool_call_denied.saturating_add(1)
                                }
                            }

                            self.emit_tool_result(
                                tx,
                                &tc.id,
                                &tc.name,
                                tool_result.output.clone(),
                                tool_result.status.clone(),
                                None,
                                false,
                            );

                            self.push_tool_history(
                                history,
                                &tc.id,
                                tool_result.output,
                                tool_result.status,
                            );
                        }
                    }
                    Err(e) => {
                        let error_output = format!("Tool execution error: {}", e);
                        tool_call_failed = tool_call_failed.saturating_add(1);

                        self.emit_tool_result(
                            tx,
                            &tc.id,
                            &tc.name,
                            error_output.clone(),
                            ToolCallStatus::Failed,
                            None,
                            false,
                        );
                        self.push_tool_history(
                            history,
                            &tc.id,
                            error_output,
                            ToolCallStatus::Failed,
                        );
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

    fn emit_tool_result(
        &self,
        tx: &mpsc::UnboundedSender<AgentEvent>,
        call_id: &str,
        name: &str,
        output: String,
        status: ToolCallStatus,
        approval_request_id: Option<String>,
        awaiting_approval: bool,
    ) {
        let _ = tx.send(AgentEvent::ToolCallResult {
            call_id: call_id.to_string(),
            name: name.to_string(),
            success: status == ToolCallStatus::Success,
            output,
            status,
            approval_request_id,
            awaiting_approval,
        });
    }

    fn push_tool_history(
        &self,
        history: &mut Vec<ChatMessage>,
        call_id: &str,
        output: String,
        status: ToolCallStatus,
    ) {
        let tool_msg_content = serde_json::json!({
            "tool_call_id": call_id,
            "content": output,
            "status": status,
        })
        .to_string();

        history.push(ChatMessage::tool(&tool_msg_content));
    }

    async fn await_tool_approval_decision(
        &self,
        approval_rx: &mut mpsc::UnboundedReceiver<ToolApprovalDecision>,
        request_id: &str,
    ) -> Option<ToolApprovalDecisionKind> {
        let timeout = tokio::time::sleep(TOOL_APPROVAL_TIMEOUT);
        tokio::pin!(timeout);

        loop {
            tokio::select! {
                _ = &mut timeout => {
                    return None;
                }
                maybe_decision = approval_rx.recv() => {
                    let decision = maybe_decision?;
                    if decision.request_id == request_id {
                        return Some(decision.decision);
                    }
                }
            }
        }
    }
}

fn suggested_allow_rule(tool_name: &str, args: &serde_json::Value) -> String {
    if let Some((server_prefix, _tool_name)) = tool_name.split_once("__") {
        if let Some(stripped) = server_prefix.strip_prefix("mcp_") {
            return format!("mcp_{}_*(*)", stripped);
        }
    }

    if tool_name.eq_ignore_ascii_case("shell") {
        let command = args
            .get("command")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("")
            .trim();
        if command.is_empty() {
            return "shell(*)".to_string();
        }

        let mut tokens = command.split_whitespace();
        let first = tokens.next().unwrap_or_default();
        let second = tokens.next();
        if let Some(second) = second {
            return format!("shell(command:{} {} *)", first, second);
        }
        return format!("shell(command:{} *)", first);
    }

    if tool_name.eq_ignore_ascii_case("filesystem_read") {
        let path = args
            .get("path")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("")
            .trim();
        if !path.is_empty() {
            let normalized = path.replace('\\', "/");
            if let Some((dir, _)) = normalized.rsplit_once('/') {
                if !dir.is_empty() {
                    return format!("filesystem_read(path:{}/*)", dir);
                }
            }
        }
    }

    format!("{}(*)", tool_name)
}

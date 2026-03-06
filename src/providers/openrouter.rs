use super::{ChatMessage, ChatResponse, Provider, TokenUsage, ToolCall, ToolSpec};
use anyhow::Result;
use async_trait::async_trait;
use futures_util::StreamExt;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tokio::sync::mpsc;

// ── Public provider struct ──────────────────────────────────────────────────

pub struct OpenRouterProvider {
    api_key: String,
    client: Client,
}

impl OpenRouterProvider {
    pub fn new(api_key: &str) -> Self {
        let client = Client::builder()
            .connect_timeout(Duration::from_secs(15))
            .timeout(Duration::from_secs(120))
            .build()
            .unwrap_or_else(|_| Client::new());

        Self {
            api_key: api_key.to_string(),
            client,
        }
    }
}

// ── OpenAI-compatible request types ─────────────────────────────────────────

#[derive(Debug, Serialize)]
struct ApiChatRequest {
    model: String,
    messages: Vec<ApiMessage>,
    temperature: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    stream: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<ApiToolSpec>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_choice: Option<String>,
}

#[derive(Debug, Serialize)]
struct ApiMessage {
    role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_call_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_calls: Option<Vec<ApiToolCall>>,
}

#[derive(Debug, Serialize)]
struct ApiToolSpec {
    #[serde(rename = "type")]
    kind: String,
    function: ApiToolFunction,
}

#[derive(Debug, Serialize)]
struct ApiToolFunction {
    name: String,
    description: String,
    parameters: serde_json::Value,
}

#[derive(Debug, Serialize, Deserialize)]
struct ApiToolCall {
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<String>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    kind: Option<String>,
    function: ApiToolCallFunction,
}

#[derive(Debug, Serialize, Deserialize)]
struct ApiToolCallFunction {
    name: String,
    arguments: String,
}

// ── OpenAI-compatible response types ────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct ApiChatResponse {
    choices: Vec<ApiChoice>,
    #[serde(default)]
    usage: Option<ApiUsage>,
}

#[derive(Debug, Deserialize)]
struct ApiChoice {
    message: ApiResponseMessage,
}

#[derive(Debug, Deserialize)]
struct ApiResponseMessage {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    tool_calls: Option<Vec<ApiToolCall>>,
}

#[derive(Debug, Deserialize)]
struct ApiUsage {
    #[serde(default)]
    prompt_tokens: Option<u32>,
    #[serde(default)]
    completion_tokens: Option<u32>,
    #[serde(default)]
    total_tokens: Option<u32>,
}

#[derive(Debug, Default)]
struct StreamToolCallAccumulator {
    id: Option<String>,
    name: String,
    arguments: String,
}

// ── Conversion helpers ──────────────────────────────────────────────────────

impl OpenRouterProvider {
    fn parse_provider_tool_calls(value: &serde_json::Value) -> Option<Vec<ToolCall>> {
        if let Ok(calls) = serde_json::from_value::<Vec<ToolCall>>(value.clone()) {
            return Some(calls);
        }

        let arr = value.as_array()?;
        let mut out = Vec::with_capacity(arr.len());

        for item in arr {
            let id = item.get("id")?.as_str()?.to_string();

            // Legacy/native OpenAI tool_call shape:
            // {"id":"...","type":"function","function":{"name":"...","arguments":"..."}}
            if let Some(function) = item.get("function") {
                let name = function.get("name")?.as_str()?.to_string();
                let arguments = function.get("arguments")?.as_str()?.to_string();
                out.push(ToolCall {
                    id,
                    name,
                    arguments,
                });
                continue;
            }

            // Flat provider shape:
            // {"id":"...","name":"...","arguments":"..."}
            let name = item.get("name")?.as_str()?.to_string();
            let arguments = item.get("arguments")?.as_str()?.to_string();
            out.push(ToolCall {
                id,
                name,
                arguments,
            });
        }

        Some(out)
    }

    /// Convert our `ToolSpec` list into the OpenAI function-calling format.
    fn convert_tools(tools: Option<&[ToolSpec]>) -> Option<Vec<ApiToolSpec>> {
        let items = tools?;
        if items.is_empty() {
            return None;
        }
        Some(
            items
                .iter()
                .map(|tool| ApiToolSpec {
                    kind: "function".to_string(),
                    function: ApiToolFunction {
                        name: tool.name.clone(),
                        description: tool.description.clone(),
                        parameters: tool.parameters.clone(),
                    },
                })
                .collect(),
        )
    }

    /// Convert `ChatMessage` list into native OpenAI API message format.
    ///
    /// Tool calls and tool results are encoded as JSON strings inside
    /// `ChatMessage.content`. Supports two encoding variants:
    ///
    /// **Object format** (zeroclaw pattern):
    /// - Assistant with tool calls: `{"tool_calls": [...], "content": "optional text"}`
    /// - Tool result: `{"tool_call_id": "...", "content": "result text"}`
    ///
    /// **Array format** (agent loop pattern):
    /// - Assistant with tool calls: `[{"id":"...","name":"...","arguments":"..."}]`
    /// - Tool result: `{"tool_call_id": "...", "result": {"output": "...", ...}}`
    fn convert_messages(messages: &[ChatMessage]) -> Vec<ApiMessage> {
        messages
            .iter()
            .map(|m| {
                // Assistant messages may contain encoded tool calls
                if m.role == "assistant" {
                    if let Ok(value) = serde_json::from_str::<serde_json::Value>(&m.content) {
                        // Object format:
                        // - {"tool_calls":[...],"content":"..."} (current)
                        // - {"__tool_calls":[...],"text":"..."} (legacy)
                        let tool_calls_value = value
                            .get("tool_calls")
                            .or_else(|| value.get("__tool_calls"));

                        if let Some(tool_calls_value) = tool_calls_value {
                            if let Some(parsed_calls) =
                                Self::parse_provider_tool_calls(tool_calls_value)
                            {
                                let api_tool_calls = Self::tool_calls_to_api(parsed_calls);
                                let content = value
                                    .get("content")
                                    .or_else(|| value.get("text"))
                                    .and_then(serde_json::Value::as_str)
                                    .map(ToString::to_string);
                                return ApiMessage {
                                    role: "assistant".to_string(),
                                    content,
                                    tool_call_id: None,
                                    tool_calls: Some(api_tool_calls),
                                };
                            }
                        }

                        // Array format: [{"id":"...","name":"...","arguments":"..."}]
                        if value.is_array() {
                            if let Some(parsed_calls) = Self::parse_provider_tool_calls(&value) {
                                let api_tool_calls = Self::tool_calls_to_api(parsed_calls);
                                return ApiMessage {
                                    role: "assistant".to_string(),
                                    content: None,
                                    tool_call_id: None,
                                    tool_calls: Some(api_tool_calls),
                                };
                            }
                        }
                    }
                }

                // Tool messages contain encoded tool result
                if m.role == "tool" {
                    if let Ok(value) = serde_json::from_str::<serde_json::Value>(&m.content) {
                        let tool_call_id = value
                            .get("tool_call_id")
                            .and_then(serde_json::Value::as_str)
                            .map(ToString::to_string);

                        // Try "content" field first, then stringify "result" field,
                        // then fall back to raw content
                        let content = value
                            .get("content")
                            .and_then(serde_json::Value::as_str)
                            .map(ToString::to_string)
                            .or_else(|| {
                                value.get("result").map(|r| {
                                    // If result is a string use it directly,
                                    // otherwise serialize the object
                                    r.as_str()
                                        .map(ToString::to_string)
                                        .unwrap_or_else(|| r.to_string())
                                })
                            })
                            .or_else(|| Some(m.content.clone()));

                        return ApiMessage {
                            role: "tool".to_string(),
                            content,
                            tool_call_id,
                            tool_calls: None,
                        };
                    }
                }

                // Regular message (system, user, or plain assistant)
                ApiMessage {
                    role: m.role.clone(),
                    content: Some(m.content.clone()),
                    tool_call_id: None,
                    tool_calls: None,
                }
            })
            .collect()
    }

    /// Convert parsed `ToolCall` list into API-native tool call format.
    fn tool_calls_to_api(calls: Vec<ToolCall>) -> Vec<ApiToolCall> {
        calls
            .into_iter()
            .map(|tc| ApiToolCall {
                id: Some(tc.id),
                kind: Some("function".to_string()),
                function: ApiToolCallFunction {
                    name: tc.name,
                    arguments: tc.arguments,
                },
            })
            .collect()
    }

    /// Parse the API response message into our `ChatResponse`.
    fn parse_response(message: ApiResponseMessage, usage: Option<ApiUsage>) -> ChatResponse {
        let tool_calls: Vec<ToolCall> = message
            .tool_calls
            .unwrap_or_default()
            .into_iter()
            .map(|tc| ToolCall {
                id: tc.id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
                name: tc.function.name,
                arguments: tc.function.arguments,
            })
            .collect();

        let token_usage = usage.map(|u| TokenUsage {
            prompt_tokens: u.prompt_tokens.unwrap_or(0),
            completion_tokens: u.completion_tokens.unwrap_or(0),
            total_tokens: u.total_tokens.unwrap_or(0),
        });

        ChatResponse {
            text: message.content,
            tool_calls,
            usage: token_usage,
        }
    }

    fn merge_streamed_name(existing: &mut String, incoming: &str) {
        if incoming.is_empty() {
            return;
        }

        if existing.is_empty() {
            *existing = incoming.to_string();
            return;
        }

        if incoming.starts_with(existing.as_str()) {
            *existing = incoming.to_string();
            return;
        }

        if !existing.ends_with(incoming) {
            existing.push_str(incoming);
        }
    }

    fn parse_sse_payload(
        payload: &str,
        full_text: &mut String,
        streamed_tool_calls: &mut Vec<StreamToolCallAccumulator>,
        usage: &mut Option<ApiUsage>,
        chunk_tx: Option<&mpsc::UnboundedSender<String>>,
    ) {
        let trimmed = payload.trim();
        if trimmed.is_empty() || trimmed == "[DONE]" {
            return;
        }

        let value = match serde_json::from_str::<serde_json::Value>(trimmed) {
            Ok(value) => value,
            Err(err) => {
                tracing::debug!("ignoring malformed OpenRouter SSE payload: {}", err);
                return;
            }
        };

        if let Some(raw_usage) = value.get("usage") {
            if let Ok(parsed_usage) = serde_json::from_value::<ApiUsage>(raw_usage.clone()) {
                *usage = Some(parsed_usage);
            }
        }

        let Some(choices) = value.get("choices").and_then(serde_json::Value::as_array) else {
            return;
        };

        for choice in choices {
            let Some(delta) = choice.get("delta").and_then(serde_json::Value::as_object) else {
                continue;
            };

            if let Some(content) = delta.get("content").and_then(serde_json::Value::as_str) {
                if !content.is_empty() {
                    full_text.push_str(content);
                    if let Some(tx) = chunk_tx {
                        let _ = tx.send(content.to_string());
                    }
                }
            }

            let Some(tool_calls) = delta
                .get("tool_calls")
                .and_then(serde_json::Value::as_array)
            else {
                continue;
            };

            for tool_call_delta in tool_calls {
                let Some(delta_obj) = tool_call_delta.as_object() else {
                    continue;
                };

                let index = delta_obj
                    .get("index")
                    .and_then(serde_json::Value::as_u64)
                    .and_then(|raw| usize::try_from(raw).ok())
                    .unwrap_or(streamed_tool_calls.len());

                while streamed_tool_calls.len() <= index {
                    streamed_tool_calls.push(StreamToolCallAccumulator::default());
                }

                let acc = &mut streamed_tool_calls[index];

                if let Some(id) = delta_obj.get("id").and_then(serde_json::Value::as_str) {
                    if !id.is_empty() {
                        acc.id = Some(id.to_string());
                    }
                }

                if let Some(function) = delta_obj
                    .get("function")
                    .and_then(serde_json::Value::as_object)
                {
                    if let Some(name_piece) =
                        function.get("name").and_then(serde_json::Value::as_str)
                    {
                        Self::merge_streamed_name(&mut acc.name, name_piece);
                    }

                    if let Some(arguments_piece) = function
                        .get("arguments")
                        .and_then(serde_json::Value::as_str)
                    {
                        acc.arguments.push_str(arguments_piece);
                    }
                }
            }
        }
    }

    fn streamed_tool_calls_to_response(
        streamed_tool_calls: Vec<StreamToolCallAccumulator>,
    ) -> Vec<ToolCall> {
        streamed_tool_calls
            .into_iter()
            .filter(|entry| {
                entry.id.is_some()
                    || !entry.name.trim().is_empty()
                    || !entry.arguments.trim().is_empty()
            })
            .map(|entry| ToolCall {
                id: entry.id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
                name: if entry.name.trim().is_empty() {
                    "unknown_tool".to_string()
                } else {
                    entry.name
                },
                arguments: if entry.arguments.trim().is_empty() {
                    "{}".to_string()
                } else {
                    entry.arguments
                },
            })
            .collect()
    }
}

// ── Provider trait implementation ───────────────────────────────────────────

#[async_trait]
impl Provider for OpenRouterProvider {
    fn supports_native_tools(&self) -> bool {
        true
    }

    async fn chat(
        &self,
        messages: &[ChatMessage],
        tools: Option<&[ToolSpec]>,
        model: &str,
        temperature: f64,
    ) -> Result<ChatResponse> {
        let api_tools = Self::convert_tools(tools);
        let api_messages = Self::convert_messages(messages);

        let request = ApiChatRequest {
            model: model.to_string(),
            messages: api_messages,
            temperature,
            stream: None,
            tool_choice: api_tools.as_ref().map(|_| "auto".to_string()),
            tools: api_tools,
        };

        let response = self
            .client
            .post("https://openrouter.ai/api/v1/chat/completions")
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("HTTP-Referer", "https://github.com/jaehong21/rikabot")
            .header("X-Title", "Rikabot")
            .json(&request)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response
                .text()
                .await
                .unwrap_or_else(|_| "<failed to read body>".to_string());
            anyhow::bail!("OpenRouter API error ({}): {}", status, body);
        }

        let api_response: ApiChatResponse = response.json().await?;

        let choice = api_response
            .choices
            .into_iter()
            .next()
            .ok_or_else(|| anyhow::anyhow!("No choices returned from OpenRouter"))?;

        Ok(Self::parse_response(choice.message, api_response.usage))
    }

    async fn chat_with_chunks(
        &self,
        messages: &[ChatMessage],
        tools: Option<&[ToolSpec]>,
        model: &str,
        temperature: f64,
        chunk_tx: Option<mpsc::UnboundedSender<String>>,
    ) -> Result<ChatResponse> {
        let api_tools = Self::convert_tools(tools);
        let api_messages = Self::convert_messages(messages);

        let request = ApiChatRequest {
            model: model.to_string(),
            messages: api_messages,
            temperature,
            stream: Some(true),
            tool_choice: api_tools.as_ref().map(|_| "auto".to_string()),
            tools: api_tools,
        };

        let response = self
            .client
            .post("https://openrouter.ai/api/v1/chat/completions")
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("HTTP-Referer", "https://github.com/jaehong21/rikabot")
            .header("X-Title", "Rikabot")
            .header("Accept", "text/event-stream")
            .json(&request)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response
                .text()
                .await
                .unwrap_or_else(|_| "<failed to read body>".to_string());
            anyhow::bail!("OpenRouter API error ({}): {}", status, body);
        }

        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("")
            .to_ascii_lowercase();

        if !content_type.starts_with("text/event-stream") {
            let api_response: ApiChatResponse = response.json().await?;
            let choice = api_response
                .choices
                .into_iter()
                .next()
                .ok_or_else(|| anyhow::anyhow!("No choices returned from OpenRouter"))?;
            let parsed = Self::parse_response(choice.message, api_response.usage);
            if let Some(tx) = chunk_tx {
                if let Some(text) = parsed.text.clone() {
                    if !text.is_empty() {
                        let _ = tx.send(text);
                    }
                }
            }
            return Ok(parsed);
        }

        let mut stream = response.bytes_stream();
        let mut line_buffer = String::new();
        let mut data_lines: Vec<String> = Vec::new();
        let mut full_text = String::new();
        let mut streamed_tool_calls: Vec<StreamToolCallAccumulator> = Vec::new();
        let mut usage: Option<ApiUsage> = None;

        while let Some(chunk_result) = stream.next().await {
            let bytes = chunk_result?;
            let decoded = String::from_utf8_lossy(&bytes);
            line_buffer.push_str(&decoded);

            while let Some(newline_idx) = line_buffer.find('\n') {
                let mut line = line_buffer[..newline_idx].to_string();
                line_buffer.drain(..=newline_idx);

                if line.ends_with('\r') {
                    line.pop();
                }

                if line.is_empty() {
                    if !data_lines.is_empty() {
                        let payload = data_lines.join("\n");
                        Self::parse_sse_payload(
                            &payload,
                            &mut full_text,
                            &mut streamed_tool_calls,
                            &mut usage,
                            chunk_tx.as_ref(),
                        );
                        data_lines.clear();
                    }
                    continue;
                }

                if let Some(data) = line.strip_prefix("data:") {
                    data_lines.push(data.trim_start().to_string());
                }
            }
        }

        if !line_buffer.is_empty() {
            let mut trailing = line_buffer;
            if trailing.ends_with('\r') {
                trailing.pop();
            }
            if let Some(data) = trailing.strip_prefix("data:") {
                data_lines.push(data.trim_start().to_string());
            }
        }

        if !data_lines.is_empty() {
            let payload = data_lines.join("\n");
            Self::parse_sse_payload(
                &payload,
                &mut full_text,
                &mut streamed_tool_calls,
                &mut usage,
                chunk_tx.as_ref(),
            );
        }

        Ok(ChatResponse {
            text: if full_text.is_empty() {
                None
            } else {
                Some(full_text)
            },
            tool_calls: Self::streamed_tool_calls_to_response(streamed_tool_calls),
            usage: usage.map(|u| TokenUsage {
                prompt_tokens: u.prompt_tokens.unwrap_or(0),
                completion_tokens: u.completion_tokens.unwrap_or(0),
                total_tokens: u.total_tokens.unwrap_or(0),
            }),
        })
    }
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::mpsc;

    #[test]
    fn provider_supports_native_tools() {
        let provider = OpenRouterProvider::new("test-key");
        assert!(provider.supports_native_tools());
    }

    #[test]
    fn convert_tools_none_when_none() {
        assert!(OpenRouterProvider::convert_tools(None).is_none());
    }

    #[test]
    fn convert_tools_none_when_empty() {
        assert!(OpenRouterProvider::convert_tools(Some(&[])).is_none());
    }

    #[test]
    fn convert_tools_produces_openai_format() {
        let tools = vec![ToolSpec {
            name: "shell".to_string(),
            description: "Run a shell command".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "command": { "type": "string" }
                },
                "required": ["command"]
            }),
        }];

        let converted = OpenRouterProvider::convert_tools(Some(&tools)).unwrap();
        assert_eq!(converted.len(), 1);
        assert_eq!(converted[0].kind, "function");
        assert_eq!(converted[0].function.name, "shell");
        assert_eq!(converted[0].function.description, "Run a shell command");
    }

    #[test]
    fn convert_messages_plain_user() {
        let messages = vec![ChatMessage::user("Hello")];
        let converted = OpenRouterProvider::convert_messages(&messages);
        assert_eq!(converted.len(), 1);
        assert_eq!(converted[0].role, "user");
        assert_eq!(converted[0].content.as_deref(), Some("Hello"));
        assert!(converted[0].tool_calls.is_none());
        assert!(converted[0].tool_call_id.is_none());
    }

    #[test]
    fn convert_messages_system_and_user() {
        let messages = vec![ChatMessage::system("Be helpful"), ChatMessage::user("Hi")];
        let converted = OpenRouterProvider::convert_messages(&messages);
        assert_eq!(converted.len(), 2);
        assert_eq!(converted[0].role, "system");
        assert_eq!(converted[0].content.as_deref(), Some("Be helpful"));
        assert_eq!(converted[1].role, "user");
    }

    #[test]
    fn convert_messages_parses_assistant_tool_calls() {
        let content = serde_json::json!({
            "content": "Let me check that",
            "tool_calls": [{
                "id": "call_123",
                "name": "shell",
                "arguments": "{\"command\":\"pwd\"}"
            }]
        });
        let messages = vec![ChatMessage::assistant(&content.to_string())];
        let converted = OpenRouterProvider::convert_messages(&messages);

        assert_eq!(converted.len(), 1);
        assert_eq!(converted[0].role, "assistant");
        assert_eq!(converted[0].content.as_deref(), Some("Let me check that"));

        let tool_calls = converted[0].tool_calls.as_ref().unwrap();
        assert_eq!(tool_calls.len(), 1);
        assert_eq!(tool_calls[0].id.as_deref(), Some("call_123"));
        assert_eq!(tool_calls[0].kind.as_deref(), Some("function"));
        assert_eq!(tool_calls[0].function.name, "shell");
        assert_eq!(tool_calls[0].function.arguments, "{\"command\":\"pwd\"}");
    }

    #[test]
    fn convert_messages_parses_tool_result() {
        let content = serde_json::json!({
            "tool_call_id": "call_123",
            "content": "/home/user"
        });
        let messages = vec![ChatMessage::tool(&content.to_string())];
        let converted = OpenRouterProvider::convert_messages(&messages);

        assert_eq!(converted.len(), 1);
        assert_eq!(converted[0].role, "tool");
        assert_eq!(converted[0].tool_call_id.as_deref(), Some("call_123"));
        assert_eq!(converted[0].content.as_deref(), Some("/home/user"));
        assert!(converted[0].tool_calls.is_none());
    }

    #[test]
    fn convert_messages_parses_assistant_tool_calls_array_format() {
        // The agent loop stores tool calls as a bare JSON array
        let tool_calls = serde_json::json!([{
            "id": "call_789",
            "name": "shell",
            "arguments": "{\"command\":\"ls -la\"}"
        }]);
        let messages = vec![ChatMessage::assistant(&tool_calls.to_string())];
        let converted = OpenRouterProvider::convert_messages(&messages);

        assert_eq!(converted.len(), 1);
        assert_eq!(converted[0].role, "assistant");
        assert!(converted[0].content.is_none()); // no text in array format

        let api_tool_calls = converted[0].tool_calls.as_ref().unwrap();
        assert_eq!(api_tool_calls.len(), 1);
        assert_eq!(api_tool_calls[0].id.as_deref(), Some("call_789"));
        assert_eq!(api_tool_calls[0].function.name, "shell");
        assert_eq!(
            api_tool_calls[0].function.arguments,
            "{\"command\":\"ls -la\"}"
        );
    }

    #[test]
    fn convert_messages_parses_legacy_assistant_tool_calls_format() {
        let content = serde_json::json!({
            "__tool_calls": [{
                "id": "call_legacy",
                "type": "function",
                "function": {
                    "name": "filesystem_read",
                    "arguments": "{\"path\":\"./workspace/skills/github/SKILL.md\"}"
                }
            }],
            "text": "reading skill..."
        });
        let messages = vec![ChatMessage::assistant(&content.to_string())];
        let converted = OpenRouterProvider::convert_messages(&messages);

        assert_eq!(converted.len(), 1);
        assert_eq!(converted[0].role, "assistant");
        assert_eq!(converted[0].content.as_deref(), Some("reading skill..."));

        let tool_calls = converted[0]
            .tool_calls
            .as_ref()
            .expect("legacy tool calls should be parsed");
        assert_eq!(tool_calls.len(), 1);
        assert_eq!(tool_calls[0].id.as_deref(), Some("call_legacy"));
        assert_eq!(tool_calls[0].function.name, "filesystem_read");
        assert_eq!(
            tool_calls[0].function.arguments,
            "{\"path\":\"./workspace/skills/github/SKILL.md\"}"
        );
    }

    #[test]
    fn convert_messages_parses_tool_result_with_result_field() {
        // The agent loop stores tool results with a "result" field instead of "content"
        let content = serde_json::json!({
            "tool_call_id": "call_789",
            "result": {
                "success": true,
                "output": "file1.txt\nfile2.txt",
                "error": null
            }
        });
        let messages = vec![ChatMessage::tool(&content.to_string())];
        let converted = OpenRouterProvider::convert_messages(&messages);

        assert_eq!(converted.len(), 1);
        assert_eq!(converted[0].role, "tool");
        assert_eq!(converted[0].tool_call_id.as_deref(), Some("call_789"));
        // The result object should be serialized as a string
        let content_str = converted[0].content.as_ref().unwrap();
        assert!(content_str.contains("file1.txt"));
        assert!(content_str.contains("success"));
    }

    #[test]
    fn convert_messages_plain_assistant_no_tool_calls() {
        let messages = vec![ChatMessage::assistant("Just a text response")];
        let converted = OpenRouterProvider::convert_messages(&messages);

        assert_eq!(converted.len(), 1);
        assert_eq!(converted[0].role, "assistant");
        assert_eq!(
            converted[0].content.as_deref(),
            Some("Just a text response")
        );
        assert!(converted[0].tool_calls.is_none());
    }

    #[test]
    fn parse_response_text_only() {
        let message = ApiResponseMessage {
            content: Some("Hello there".to_string()),
            tool_calls: None,
        };
        let response = OpenRouterProvider::parse_response(message, None);

        assert_eq!(response.text.as_deref(), Some("Hello there"));
        assert!(response.tool_calls.is_empty());
        assert!(response.usage.is_none());
    }

    #[test]
    fn parse_response_with_tool_calls() {
        let message = ApiResponseMessage {
            content: Some("Let me run that".to_string()),
            tool_calls: Some(vec![ApiToolCall {
                id: Some("call_abc".to_string()),
                kind: Some("function".to_string()),
                function: ApiToolCallFunction {
                    name: "shell".to_string(),
                    arguments: r#"{"command":"ls"}"#.to_string(),
                },
            }]),
        };
        let usage = Some(ApiUsage {
            prompt_tokens: Some(100),
            completion_tokens: Some(50),
            total_tokens: Some(150),
        });

        let response = OpenRouterProvider::parse_response(message, usage);

        assert_eq!(response.text.as_deref(), Some("Let me run that"));
        assert_eq!(response.tool_calls.len(), 1);
        assert_eq!(response.tool_calls[0].id, "call_abc");
        assert_eq!(response.tool_calls[0].name, "shell");
        assert_eq!(response.tool_calls[0].arguments, r#"{"command":"ls"}"#);

        let token_usage = response.usage.unwrap();
        assert_eq!(token_usage.prompt_tokens, 100);
        assert_eq!(token_usage.completion_tokens, 50);
        assert_eq!(token_usage.total_tokens, 150);
    }

    #[test]
    fn parse_response_tool_call_without_id_gets_uuid() {
        let message = ApiResponseMessage {
            content: None,
            tool_calls: Some(vec![ApiToolCall {
                id: None,
                kind: None,
                function: ApiToolCallFunction {
                    name: "shell".to_string(),
                    arguments: "{}".to_string(),
                },
            }]),
        };
        let response = OpenRouterProvider::parse_response(message, None);

        assert_eq!(response.tool_calls.len(), 1);
        // UUID should be a non-empty string
        assert!(!response.tool_calls[0].id.is_empty());
    }

    #[test]
    fn api_chat_request_serializes_correctly() {
        let request = ApiChatRequest {
            model: "anthropic/claude-sonnet-4".to_string(),
            messages: vec![ApiMessage {
                role: "user".to_string(),
                content: Some("Hello".to_string()),
                tool_call_id: None,
                tool_calls: None,
            }],
            temperature: 0.7,
            stream: None,
            tools: None,
            tool_choice: None,
        };

        let json = serde_json::to_string(&request).unwrap();
        assert!(json.contains("anthropic/claude-sonnet-4"));
        assert!(json.contains("\"role\":\"user\""));
        assert!(json.contains("\"temperature\":0.7"));
        assert!(!json.contains("\"stream\""));
        // tools and tool_choice should be omitted when None
        assert!(!json.contains("\"tools\""));
        assert!(!json.contains("\"tool_choice\""));
    }

    #[test]
    fn api_chat_request_serializes_with_tools() {
        let request = ApiChatRequest {
            model: "openai/gpt-4o".to_string(),
            messages: vec![ApiMessage {
                role: "user".to_string(),
                content: Some("Check the date".to_string()),
                tool_call_id: None,
                tool_calls: None,
            }],
            temperature: 0.1,
            stream: None,
            tools: Some(vec![ApiToolSpec {
                kind: "function".to_string(),
                function: ApiToolFunction {
                    name: "shell".to_string(),
                    description: "Run a command".to_string(),
                    parameters: serde_json::json!({"type": "object"}),
                },
            }]),
            tool_choice: Some("auto".to_string()),
        };

        let json = serde_json::to_string(&request).unwrap();
        assert!(json.contains("\"tools\""));
        assert!(json.contains("\"tool_choice\":\"auto\""));
        assert!(json.contains("\"type\":\"function\""));
        assert!(json.contains("\"name\":\"shell\""));
    }

    #[test]
    fn api_chat_request_serializes_stream_true() {
        let request = ApiChatRequest {
            model: "openai/gpt-4o".to_string(),
            messages: vec![ApiMessage {
                role: "user".to_string(),
                content: Some("hello".to_string()),
                tool_call_id: None,
                tool_calls: None,
            }],
            temperature: 0.0,
            stream: Some(true),
            tools: None,
            tool_choice: None,
        };

        let json = serde_json::to_string(&request).unwrap();
        assert!(json.contains("\"stream\":true"));
    }

    #[test]
    fn api_response_deserializes_text_only() {
        let json = r#"{"choices":[{"message":{"content":"Hello from OpenRouter"}}]}"#;
        let response: ApiChatResponse = serde_json::from_str(json).unwrap();

        assert_eq!(response.choices.len(), 1);
        assert_eq!(
            response.choices[0].message.content.as_deref(),
            Some("Hello from OpenRouter")
        );
        assert!(response.choices[0].message.tool_calls.is_none());
        assert!(response.usage.is_none());
    }

    #[test]
    fn api_response_deserializes_with_tool_calls() {
        let json = r#"{
            "choices":[{
                "message":{
                    "content":null,
                    "tool_calls":[{
                        "id":"call_456",
                        "type":"function",
                        "function":{"name":"shell","arguments":"{\"command\":\"date\"}"}
                    }]
                }
            }],
            "usage":{"prompt_tokens":42,"completion_tokens":15,"total_tokens":57}
        }"#;

        let response: ApiChatResponse = serde_json::from_str(json).unwrap();
        assert_eq!(response.choices.len(), 1);
        assert!(response.choices[0].message.content.is_none());

        let tool_calls = response.choices[0].message.tool_calls.as_ref().unwrap();
        assert_eq!(tool_calls.len(), 1);
        assert_eq!(tool_calls[0].id.as_deref(), Some("call_456"));
        assert_eq!(tool_calls[0].function.name, "shell");

        let usage = response.usage.unwrap();
        assert_eq!(usage.prompt_tokens, Some(42));
        assert_eq!(usage.completion_tokens, Some(15));
        assert_eq!(usage.total_tokens, Some(57));
    }

    #[test]
    fn api_response_deserializes_empty_choices() {
        let json = r#"{"choices":[]}"#;
        let response: ApiChatResponse = serde_json::from_str(json).unwrap();
        assert!(response.choices.is_empty());
    }

    #[test]
    fn parse_sse_payload_streams_text_and_usage() {
        let (tx, mut rx) = mpsc::unbounded_channel::<String>();
        let mut full_text = String::new();
        let mut tool_calls = Vec::<StreamToolCallAccumulator>::new();
        let mut usage = None;

        OpenRouterProvider::parse_sse_payload(
            r#"{"choices":[{"delta":{"content":"open "}}],"usage":{"prompt_tokens":4,"completion_tokens":3,"total_tokens":7}}"#,
            &mut full_text,
            &mut tool_calls,
            &mut usage,
            Some(&tx),
        );
        OpenRouterProvider::parse_sse_payload(
            r#"{"choices":[{"delta":{"content":"router"}}]}"#,
            &mut full_text,
            &mut tool_calls,
            &mut usage,
            Some(&tx),
        );

        assert_eq!(full_text, "open router");
        assert_eq!(rx.try_recv().ok().as_deref(), Some("open "));
        assert_eq!(rx.try_recv().ok().as_deref(), Some("router"));
        assert!(rx.try_recv().is_err());

        let parsed_usage = usage.expect("usage should be parsed");
        assert_eq!(parsed_usage.prompt_tokens, Some(4));
        assert_eq!(parsed_usage.completion_tokens, Some(3));
        assert_eq!(parsed_usage.total_tokens, Some(7));
    }

    #[test]
    fn parse_sse_payload_accumulates_tool_call_deltas() {
        let mut full_text = String::new();
        let mut tool_calls = Vec::<StreamToolCallAccumulator>::new();
        let mut usage = None;

        OpenRouterProvider::parse_sse_payload(
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_or","function":{"name":"shell","arguments":"{\"command\":\"printf"}}]}}]}"#,
            &mut full_text,
            &mut tool_calls,
            &mut usage,
            None,
        );
        OpenRouterProvider::parse_sse_payload(
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":" done\"}"}}]}}]}"#,
            &mut full_text,
            &mut tool_calls,
            &mut usage,
            None,
        );

        let converted = OpenRouterProvider::streamed_tool_calls_to_response(tool_calls);
        assert_eq!(converted.len(), 1);
        assert_eq!(converted[0].id, "call_or");
        assert_eq!(converted[0].name, "shell");
        assert_eq!(converted[0].arguments, "{\"command\":\"printf done\"}");
    }

    #[test]
    fn parse_sse_payload_ignores_malformed_and_done_lines() {
        let mut full_text = String::new();
        let mut tool_calls = Vec::<StreamToolCallAccumulator>::new();
        let mut usage = None;

        OpenRouterProvider::parse_sse_payload(
            "not-json",
            &mut full_text,
            &mut tool_calls,
            &mut usage,
            None,
        );
        OpenRouterProvider::parse_sse_payload(
            "[DONE]",
            &mut full_text,
            &mut tool_calls,
            &mut usage,
            None,
        );

        assert!(full_text.is_empty());
        assert!(tool_calls.is_empty());
        assert!(usage.is_none());
    }
}

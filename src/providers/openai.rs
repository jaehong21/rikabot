use super::{ChatMessage, ChatResponse, Provider, TokenUsage, ToolCall, ToolSpec};
use anyhow::Result;
use async_trait::async_trait;
use futures_util::StreamExt;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tokio::sync::mpsc;

pub struct OpenAiProvider {
    api_key: String,
    base_url: String,
    client: Client,
}

impl OpenAiProvider {
    pub fn new(api_key: &str, base_url: &str) -> Self {
        let client = Client::builder()
            .connect_timeout(Duration::from_secs(15))
            .timeout(Duration::from_secs(120))
            .build()
            .unwrap_or_else(|_| Client::new());

        Self {
            api_key: api_key.to_string(),
            base_url: base_url.trim_end_matches('/').to_string(),
            client,
        }
    }

    fn chat_completions_url(&self) -> String {
        format!("{}/chat/completions", self.base_url)
    }
}

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

impl OpenAiProvider {
    fn parse_provider_tool_calls(value: &serde_json::Value) -> Option<Vec<ToolCall>> {
        if let Ok(calls) = serde_json::from_value::<Vec<ToolCall>>(value.clone()) {
            return Some(calls);
        }

        let arr = value.as_array()?;
        let mut out = Vec::with_capacity(arr.len());

        for item in arr {
            let id = item.get("id")?.as_str()?.to_string();

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

    fn convert_messages(messages: &[ChatMessage]) -> Vec<ApiMessage> {
        messages
            .iter()
            .map(|m| {
                if m.role == "assistant" {
                    if let Ok(value) = serde_json::from_str::<serde_json::Value>(&m.content) {
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

                if m.role == "tool" {
                    if let Ok(value) = serde_json::from_str::<serde_json::Value>(&m.content) {
                        let tool_call_id = value
                            .get("tool_call_id")
                            .and_then(serde_json::Value::as_str)
                            .map(ToString::to_string);

                        let content = value
                            .get("content")
                            .and_then(serde_json::Value::as_str)
                            .map(ToString::to_string)
                            .or_else(|| {
                                value.get("result").map(|r| {
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

                ApiMessage {
                    role: m.role.clone(),
                    content: Some(m.content.clone()),
                    tool_call_id: None,
                    tool_calls: None,
                }
            })
            .collect()
    }

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
                tracing::debug!("ignoring malformed SSE payload: {}", err);
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

#[async_trait]
impl Provider for OpenAiProvider {
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
            .post(self.chat_completions_url())
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&request)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response
                .text()
                .await
                .unwrap_or_else(|_| "<failed to read body>".to_string());
            anyhow::bail!("OpenAI API error ({}): {}", status, body);
        }

        let api_response: ApiChatResponse = response.json().await?;

        let choice = api_response
            .choices
            .into_iter()
            .next()
            .ok_or_else(|| anyhow::anyhow!("No choices returned from OpenAI"))?;

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
            .post(self.chat_completions_url())
            .header("Authorization", format!("Bearer {}", self.api_key))
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
            anyhow::bail!("OpenAI API error ({}): {}", status, body);
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
                .ok_or_else(|| anyhow::anyhow!("No choices returned from OpenAI"))?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::mpsc;

    #[test]
    fn provider_supports_native_tools() {
        let provider = OpenAiProvider::new("test-key", "https://api.openai.com/v1");
        assert!(provider.supports_native_tools());
    }

    #[test]
    fn chat_completions_url_uses_base_url() {
        let provider = OpenAiProvider::new("test-key", "https://example.com/v1");
        assert_eq!(
            provider.chat_completions_url(),
            "https://example.com/v1/chat/completions"
        );
    }

    #[test]
    fn chat_completions_url_trims_trailing_slash() {
        let provider = OpenAiProvider::new("test-key", "https://example.com/v1/");
        assert_eq!(
            provider.chat_completions_url(),
            "https://example.com/v1/chat/completions"
        );
    }

    #[test]
    fn api_chat_request_omits_stream_when_none() {
        let request = ApiChatRequest {
            model: "gpt-5.2".to_string(),
            messages: vec![ApiMessage {
                role: "user".to_string(),
                content: Some("Hello".to_string()),
                tool_call_id: None,
                tool_calls: None,
            }],
            temperature: 0.0,
            stream: None,
            tools: None,
            tool_choice: None,
        };

        let json = serde_json::to_string(&request).unwrap();
        assert!(!json.contains("\"stream\""));
    }

    #[test]
    fn api_chat_request_serializes_stream_true() {
        let request = ApiChatRequest {
            model: "gpt-5.2".to_string(),
            messages: vec![ApiMessage {
                role: "user".to_string(),
                content: Some("Hello".to_string()),
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
    fn convert_tools_none_when_none() {
        assert!(OpenAiProvider::convert_tools(None).is_none());
    }

    #[test]
    fn convert_tools_none_when_empty() {
        assert!(OpenAiProvider::convert_tools(Some(&[])).is_none());
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

        let converted = OpenAiProvider::convert_tools(Some(&tools)).unwrap();
        assert_eq!(converted.len(), 1);
        assert_eq!(converted[0].kind, "function");
        assert_eq!(converted[0].function.name, "shell");
    }

    #[test]
    fn convert_messages_plain_user() {
        let messages = vec![ChatMessage::user("Hello")];
        let converted = OpenAiProvider::convert_messages(&messages);
        assert_eq!(converted.len(), 1);
        assert_eq!(converted[0].role, "user");
        assert_eq!(converted[0].content.as_deref(), Some("Hello"));
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
        let converted = OpenAiProvider::convert_messages(&messages);

        assert_eq!(converted.len(), 1);
        assert_eq!(converted[0].role, "assistant");
        assert_eq!(converted[0].content.as_deref(), Some("Let me check that"));
        assert!(converted[0].tool_calls.is_some());
        assert_eq!(converted[0].tool_calls.as_ref().unwrap().len(), 1);
        assert_eq!(
            converted[0].tool_calls.as_ref().unwrap()[0].id.as_deref(),
            Some("call_123")
        );
    }

    #[test]
    fn convert_messages_parses_tool_result_object() {
        let tool_msg = serde_json::json!({
            "tool_call_id": "call_abc",
            "result": { "output": "ok" }
        });
        let messages = vec![ChatMessage::tool(&tool_msg.to_string())];
        let converted = OpenAiProvider::convert_messages(&messages);

        assert_eq!(converted.len(), 1);
        assert_eq!(converted[0].role, "tool");
        assert_eq!(converted[0].tool_call_id.as_deref(), Some("call_abc"));
        assert_eq!(converted[0].content.as_deref(), Some(r#"{"output":"ok"}"#));
    }

    #[test]
    fn parse_response_maps_usage_and_tool_calls() {
        let response = ApiResponseMessage {
            content: Some("done".to_string()),
            tool_calls: Some(vec![ApiToolCall {
                id: Some("call_1".to_string()),
                kind: Some("function".to_string()),
                function: ApiToolCallFunction {
                    name: "shell".to_string(),
                    arguments: r#"{"command":"pwd"}"#.to_string(),
                },
            }]),
        };

        let usage = Some(ApiUsage {
            prompt_tokens: Some(10),
            completion_tokens: Some(4),
            total_tokens: Some(14),
        });

        let parsed = OpenAiProvider::parse_response(response, usage);
        assert_eq!(parsed.text.as_deref(), Some("done"));
        assert_eq!(parsed.tool_calls.len(), 1);
        assert_eq!(parsed.tool_calls[0].id, "call_1");
        assert_eq!(parsed.tool_calls[0].name, "shell");
        assert_eq!(
            parsed.tool_calls[0].arguments,
            r#"{"command":"pwd"}"#.to_string()
        );
        assert_eq!(parsed.usage.as_ref().unwrap().total_tokens, 14);
    }

    #[test]
    fn parse_sse_payload_streams_text_and_usage() {
        let (tx, mut rx) = mpsc::unbounded_channel::<String>();
        let mut full_text = String::new();
        let mut tool_calls = Vec::<StreamToolCallAccumulator>::new();
        let mut usage = None;

        OpenAiProvider::parse_sse_payload(
            r#"{"choices":[{"delta":{"content":"hello "}}],"usage":{"prompt_tokens":3,"completion_tokens":2,"total_tokens":5}}"#,
            &mut full_text,
            &mut tool_calls,
            &mut usage,
            Some(&tx),
        );
        OpenAiProvider::parse_sse_payload(
            r#"{"choices":[{"delta":{"content":"world"}}]}"#,
            &mut full_text,
            &mut tool_calls,
            &mut usage,
            Some(&tx),
        );

        assert_eq!(full_text, "hello world");
        assert_eq!(rx.try_recv().ok().as_deref(), Some("hello "));
        assert_eq!(rx.try_recv().ok().as_deref(), Some("world"));
        assert!(rx.try_recv().is_err());

        let parsed_usage = usage.expect("usage should be parsed");
        assert_eq!(parsed_usage.prompt_tokens, Some(3));
        assert_eq!(parsed_usage.completion_tokens, Some(2));
        assert_eq!(parsed_usage.total_tokens, Some(5));
    }

    #[test]
    fn parse_sse_payload_accumulates_tool_call_deltas() {
        let mut full_text = String::new();
        let mut tool_calls = Vec::<StreamToolCallAccumulator>::new();
        let mut usage = None;

        OpenAiProvider::parse_sse_payload(
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_1","function":{"name":"shell","arguments":"{\"command\":\"echo"}}]}}]}"#,
            &mut full_text,
            &mut tool_calls,
            &mut usage,
            None,
        );
        OpenAiProvider::parse_sse_payload(
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":" hello\"}"}}]}}]}"#,
            &mut full_text,
            &mut tool_calls,
            &mut usage,
            None,
        );

        let converted = OpenAiProvider::streamed_tool_calls_to_response(tool_calls);
        assert_eq!(converted.len(), 1);
        assert_eq!(converted[0].id, "call_1");
        assert_eq!(converted[0].name, "shell");
        assert_eq!(converted[0].arguments, "{\"command\":\"echo hello\"}");
    }

    #[test]
    fn parse_sse_payload_ignores_malformed_and_done_lines() {
        let mut full_text = String::new();
        let mut tool_calls = Vec::<StreamToolCallAccumulator>::new();
        let mut usage = None;

        OpenAiProvider::parse_sse_payload(
            "not-json",
            &mut full_text,
            &mut tool_calls,
            &mut usage,
            None,
        );
        OpenAiProvider::parse_sse_payload(
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

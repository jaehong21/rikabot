use super::{ChatMessage, ChatResponse, Provider, TokenUsage, ToolCall, ToolSpec};
use anyhow::Result;
use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::Duration;

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
}

#[cfg(test)]
mod tests {
    use super::*;

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
}

use super::{Tool, ToolResult};
use crate::config::WebSearchConfig;
use anyhow::Result;
use async_trait::async_trait;
use reqwest::StatusCode;
use serde_json::json;
use std::collections::HashSet;
use std::time::Duration;

const OPENROUTER_API_BASE: &str = "https://openrouter.ai/api/v1";
const MAX_REQUEST_ATTEMPTS: usize = 2;
const RETRY_DELAY_MS: u64 = 250;

/// Search the web via OpenRouter web-search plugin.
pub struct WebSearchTool {
    config: WebSearchConfig,
    #[cfg(test)]
    openrouter_api_base_override: Option<String>,
}

impl WebSearchTool {
    pub fn new(config: WebSearchConfig) -> Self {
        Self {
            config,
            #[cfg(test)]
            openrouter_api_base_override: None,
        }
    }

    #[cfg(test)]
    fn new_with_api_base_override(config: WebSearchConfig, base: String) -> Self {
        Self {
            config,
            openrouter_api_base_override: Some(base),
        }
    }

    fn resolve_count(&self, requested: Option<u64>) -> usize {
        let from_args = requested
            .and_then(|v| usize::try_from(v).ok())
            .unwrap_or_else(|| self.config.resolved_max_results());
        from_args
            .clamp(1, 10)
            .min(self.config.resolved_max_results())
    }

    fn build_client(&self) -> Result<reqwest::Client> {
        let connect_timeout_secs = self.config.timeout_secs.min(30);
        reqwest::Client::builder()
            .timeout(Duration::from_secs(self.config.timeout_secs))
            .connect_timeout(Duration::from_secs(connect_timeout_secs))
            .user_agent(self.config.user_agent.clone())
            .build()
            .map_err(|e| anyhow::anyhow!("failed to build web_search HTTP client: {e}"))
    }

    fn should_retry_error(err: &reqwest::Error) -> bool {
        err.is_timeout() || err.is_connect() || err.is_request()
    }

    fn should_retry_status(status: StatusCode) -> bool {
        status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error()
    }

    async fn maybe_sleep_before_retry(attempt: usize) {
        if attempt < MAX_REQUEST_ATTEMPTS {
            tokio::time::sleep(Duration::from_millis(RETRY_DELAY_MS)).await;
        }
    }

    fn openrouter_api_base(&self) -> &str {
        #[cfg(test)]
        if let Some(v) = self.openrouter_api_base_override.as_deref() {
            return v;
        }
        OPENROUTER_API_BASE
    }

    fn build_openrouter_payload(
        query: &str,
        count: usize,
        model: &str,
        cfg: &crate::config::WebSearchOpenRouterConfig,
    ) -> serde_json::Value {
        let plugin_max = cfg
            .resolved_plugin_max_results()
            .unwrap_or(count)
            .min(count);
        let mut plugin = json!({
            "id": "web",
            "max_results": plugin_max,
        });
        if let Some(prompt) = cfg.resolved_plugin_search_prompt() {
            plugin["search_prompt"] = json!(prompt);
        }

        json!({
            "model": model,
            "messages": [
                {
                    "role": "user",
                    "content": query
                }
            ],
            "temperature": 0.0,
            "stream": false,
            "plugins": [plugin]
        })
    }

    async fn search_openrouter(&self, query: &str, count: usize) -> Result<String> {
        let cfg = &self.config.providers.openrouter;
        let api_key = cfg.resolve_api_key()?;
        let model = cfg.resolve_model()?;
        let endpoint = format!("{}/chat/completions", self.openrouter_api_base());
        let payload = Self::build_openrouter_payload(query, count, &model, cfg);
        let client = self.build_client()?;

        tracing::debug!(
            tool = "web_search",
            provider = "openrouter",
            query_len = query.len(),
            count,
            endpoint = %endpoint,
            model = %model,
            "starting web_search request"
        );

        for attempt in 1..=MAX_REQUEST_ATTEMPTS {
            tracing::debug!(
                tool = "web_search",
                provider = "openrouter",
                attempt,
                max_attempts = MAX_REQUEST_ATTEMPTS,
                "sending request"
            );

            let response = match client
                .post(&endpoint)
                .header(reqwest::header::AUTHORIZATION, format!("Bearer {api_key}"))
                .json(&payload)
                .send()
                .await
            {
                Ok(resp) => resp,
                Err(err) => {
                    if attempt < MAX_REQUEST_ATTEMPTS && Self::should_retry_error(&err) {
                        tracing::debug!(
                            tool = "web_search",
                            provider = "openrouter",
                            attempt,
                            error = %err,
                            "request failed with retryable error; retrying"
                        );
                        Self::maybe_sleep_before_retry(attempt).await;
                        continue;
                    }
                    anyhow::bail!("openrouter search request failed: {err}");
                }
            };

            let status = response.status();
            let text = response
                .text()
                .await
                .map_err(|e| anyhow::anyhow!("failed to read openrouter response body: {e}"))?;

            tracing::debug!(
                tool = "web_search",
                provider = "openrouter",
                attempt,
                status = %status,
                response_bytes = text.len(),
                "received response"
            );

            if !status.is_success() {
                if attempt < MAX_REQUEST_ATTEMPTS && Self::should_retry_status(status) {
                    tracing::debug!(
                        tool = "web_search",
                        provider = "openrouter",
                        attempt,
                        status = %status,
                        "response status is retryable; retrying"
                    );
                    Self::maybe_sleep_before_retry(attempt).await;
                    continue;
                }
                anyhow::bail!("openrouter search failed with status {}: {}", status, text);
            }

            let trimmed = text.trim();
            if trimmed.is_empty() {
                if attempt < MAX_REQUEST_ATTEMPTS {
                    tracing::debug!(
                        tool = "web_search",
                        provider = "openrouter",
                        attempt,
                        "response body empty; retrying"
                    );
                    Self::maybe_sleep_before_retry(attempt).await;
                    continue;
                }
                anyhow::bail!(
                    "openrouter search returned an empty body; consider increasing web_search.timeout_secs"
                );
            }

            let value: serde_json::Value = serde_json::from_str(trimmed).map_err(|e| {
                let prefix = trimmed.chars().take(160).collect::<String>();
                anyhow::anyhow!("invalid openrouter response JSON: {e}; body_prefix={prefix:?}")
            })?;
            tracing::debug!(
                tool = "web_search",
                provider = "openrouter",
                attempt,
                "parsed response JSON successfully"
            );
            return Self::format_openrouter_results(&value, query, &model);
        }

        anyhow::bail!("openrouter search failed after retry")
    }

    fn extract_urls(value: &serde_json::Value, urls: &mut Vec<String>) {
        match value {
            serde_json::Value::String(s) => {
                if s.starts_with("http://") || s.starts_with("https://") {
                    urls.push(s.to_string());
                }
            }
            serde_json::Value::Array(items) => {
                for item in items {
                    Self::extract_urls(item, urls);
                }
            }
            serde_json::Value::Object(map) => {
                for key in ["url", "href", "link"] {
                    if let Some(v) = map.get(key).and_then(|v| v.as_str()) {
                        if v.starts_with("http://") || v.starts_with("https://") {
                            urls.push(v.to_string());
                        }
                    }
                }
                for v in map.values() {
                    if v.is_array() {
                        Self::extract_urls(v, urls);
                    }
                }
            }
            _ => {}
        }
    }

    fn openrouter_message_content(value: &serde_json::Value) -> String {
        let Some(content) = value.pointer("/choices/0/message/content") else {
            return String::new();
        };

        match content {
            serde_json::Value::String(s) => s.trim().to_string(),
            serde_json::Value::Array(parts) => {
                let mut segments = Vec::new();
                for part in parts {
                    if let Some(text) = part.as_str() {
                        if !text.trim().is_empty() {
                            segments.push(text.trim().to_string());
                        }
                        continue;
                    }
                    if let Some(text) = part.get("text").and_then(|v| v.as_str()) {
                        if !text.trim().is_empty() {
                            segments.push(text.trim().to_string());
                        }
                    }
                }
                segments.join("\n")
            }
            _ => String::new(),
        }
    }

    fn format_openrouter_results(
        value: &serde_json::Value,
        query: &str,
        model: &str,
    ) -> Result<String> {
        let content = Self::openrouter_message_content(value);

        let mut urls = Vec::new();
        if let Some(citations) = value.get("citations") {
            Self::extract_urls(citations, &mut urls);
        }
        if let Some(citations) = value.pointer("/choices/0/message/citations") {
            Self::extract_urls(citations, &mut urls);
        }
        if let Some(annotations) = value.pointer("/choices/0/message/annotations") {
            Self::extract_urls(annotations, &mut urls);
        }

        let mut deduped = Vec::new();
        let mut seen = HashSet::new();
        for url in urls {
            if seen.insert(url.clone()) {
                deduped.push(url);
            }
        }

        if content.is_empty() && deduped.is_empty() {
            anyhow::bail!("openrouter response missing both content and citations");
        }

        let mut lines = vec![format!(
            "Search results for: {} (via OpenRouter:{})",
            query, model
        )];

        if !content.is_empty() {
            lines.push(String::new());
            lines.push(content);
        }

        if !deduped.is_empty() {
            lines.push(String::new());
            lines.push("Sources:".to_string());
            for (idx, url) in deduped.iter().enumerate() {
                lines.push(format!("{}. {}", idx + 1, url));
            }
        }

        Ok(lines.join("\n"))
    }
}

#[async_trait]
impl Tool for WebSearchTool {
    fn name(&self) -> &str {
        "web_search"
    }

    fn description(&self) -> &str {
        "Search the web and return relevant titles, URLs, and snippets."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Search query string."
                },
                "count": {
                    "type": "integer",
                    "description": "Maximum number of results to return (1-10)."
                }
            },
            "required": ["query"]
        })
    }

    async fn execute(&self, args: serde_json::Value) -> Result<ToolResult> {
        let query = args
            .get("query")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim();
        if query.is_empty() {
            return Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some("Missing required argument: query".to_string()),
            });
        }

        let _ = self.config.resolved_provider_kind()?;
        let count = self.resolve_count(args.get("count").and_then(|v| v.as_u64()));
        let output = self.search_openrouter(query, count).await;

        match output {
            Ok(output) => Ok(ToolResult {
                success: true,
                output,
                error: None,
            }),
            Err(err) => Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some(err.to_string()),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::extract::State;
    use axum::http::StatusCode;
    use axum::routing::post;
    use axum::{Json, Router};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use tokio::net::TcpListener;

    fn test_search_config() -> WebSearchConfig {
        let mut cfg = WebSearchConfig {
            enabled: true,
            provider: "openrouter".to_string(),
            max_results: 5,
            timeout_secs: 10,
            user_agent: "rikabot-test".to_string(),
            ..WebSearchConfig::default()
        };
        cfg.providers.openrouter.api_key = Some("or-key".to_string());
        cfg.providers.openrouter.model = Some("openai/gpt-4o-mini".to_string());
        cfg
    }

    #[test]
    fn build_openrouter_payload_includes_web_plugin() {
        let mut cfg = crate::config::WebSearchOpenRouterConfig::default();
        cfg.plugin_max_results = Some(7);
        cfg.plugin_search_prompt = Some("prefer docs pages".to_string());
        let payload =
            WebSearchTool::build_openrouter_payload("rust", 3, "openai/gpt-4o-mini", &cfg);
        assert_eq!(payload["plugins"][0]["id"], "web");
        assert_eq!(payload["plugins"][0]["max_results"], 3);
        assert_eq!(payload["plugins"][0]["search_prompt"], "prefer docs pages");
        assert_eq!(payload["stream"], false);
    }

    #[test]
    fn format_openrouter_results_dedupes_citations() {
        let body = json!({
            "choices": [
                {
                    "message": {
                        "content": "summary",
                        "citations": [
                            "https://docs.rs/a",
                            {"url":"https://docs.rs/a"},
                            {"href":"https://docs.rs/b"}
                        ]
                    }
                }
            ],
            "citations": ["https://docs.rs/c"]
        });

        let out =
            WebSearchTool::format_openrouter_results(&body, "q", "openai/gpt-4o-mini").unwrap();
        assert!(out.contains("summary"));
        assert!(out.contains("Sources:"));
        assert!(out.contains("https://docs.rs/a"));
        assert!(out.contains("https://docs.rs/b"));
        assert!(out.contains("https://docs.rs/c"));
        assert_eq!(out.matches("https://docs.rs/a").count(), 1);
    }

    #[tokio::test]
    async fn execute_openrouter_posts_model_and_plugin() {
        let captured = Arc::new(Mutex::new(None::<serde_json::Value>));
        let app_state = captured.clone();
        let app = Router::new().route(
            "/chat/completions",
            post(
                |State(state): State<Arc<Mutex<Option<serde_json::Value>>>>,
                 Json(payload): Json<serde_json::Value>| async move {
                    *state.lock().unwrap() = Some(payload);
                    Json(json!({
                        "choices": [
                            {
                                "message": {
                                    "content": "top results",
                                    "citations": ["https://openrouter.ai/docs"]
                                }
                            }
                        ]
                    }))
                },
            ),
        );
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            axum::serve(listener, app.with_state(app_state))
                .await
                .unwrap();
        });

        let mut cfg = test_search_config();
        cfg.providers.openrouter.model = Some("openai/gpt-4o-mini".to_string());
        cfg.providers.openrouter.plugin_max_results = Some(4);
        cfg.providers.openrouter.plugin_search_prompt =
            Some("prioritize official docs".to_string());
        let tool = WebSearchTool::new_with_api_base_override(cfg, format!("http://{addr}"));

        let result = tool
            .execute(json!({
                "query": "openrouter web search plugin",
                "count": 3
            }))
            .await
            .unwrap();

        server.abort();

        assert!(result.success, "expected success, got {:?}", result.error);
        assert!(result.output.contains("top results"));
        assert!(result.output.contains("https://openrouter.ai/docs"));

        let payload = captured.lock().unwrap().clone().expect("captured payload");
        assert_eq!(payload["model"], "openai/gpt-4o-mini");
        assert_eq!(payload["plugins"][0]["id"], "web");
        assert_eq!(payload["plugins"][0]["max_results"], 3);
        assert_eq!(
            payload["plugins"][0]["search_prompt"],
            "prioritize official docs"
        );
        assert_eq!(payload["stream"], false);
    }

    #[tokio::test]
    async fn execute_openrouter_retries_after_server_error() {
        let attempts = Arc::new(AtomicUsize::new(0));
        let app_state = attempts.clone();
        let app =
            Router::new().route(
                "/chat/completions",
                post(
                    |State(state): State<Arc<AtomicUsize>>,
                     Json(payload): Json<serde_json::Value>| async move {
                        let attempt = state.fetch_add(1, Ordering::SeqCst);
                        if attempt == 0 {
                            return (
                                StatusCode::SERVICE_UNAVAILABLE,
                                Json(json!({"error":"try again"})),
                            );
                        }
                        let query = payload
                            .pointer("/messages/0/content")
                            .and_then(|v| v.as_str())
                            .unwrap_or("");
                        (
                            StatusCode::OK,
                            Json(json!({
                                "choices": [{
                                    "message": {
                                        "content": format!("result for {query}"),
                                        "citations": ["https://example.com"]
                                    }
                                }]
                            })),
                        )
                    },
                ),
            );
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            axum::serve(listener, app.with_state(app_state))
                .await
                .unwrap();
        });

        let tool = WebSearchTool::new_with_api_base_override(
            test_search_config(),
            format!("http://{addr}"),
        );

        let result = tool
            .execute(json!({
                "query": "retry please",
                "count": 3
            }))
            .await
            .unwrap();

        server.abort();

        assert!(result.success, "expected success, got {:?}", result.error);
        assert!(result.output.contains("result for retry please"));
        assert!(attempts.load(Ordering::SeqCst) >= 2);
    }
}

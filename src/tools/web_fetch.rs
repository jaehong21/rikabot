use super::{Tool, ToolResult};
use crate::config::WebFetchConfig;
use anyhow::Result;
use async_trait::async_trait;
use regex::Regex;
use reqwest::redirect::Policy;
use reqwest::StatusCode;
use serde_json::json;
use std::sync::OnceLock;
use std::time::Duration;

const MAX_REDIRECTS: usize = 5;
const MAX_REQUEST_ATTEMPTS: usize = 2;
const RETRY_DELAY_MS: u64 = 250;

/// Fetch web content and return readable text/markdown.
pub struct WebFetchTool {
    config: WebFetchConfig,
    client: reqwest::Client,
}

impl WebFetchTool {
    pub fn new(config: WebFetchConfig) -> Self {
        let connect_timeout_secs = config.timeout_secs.min(30);
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(config.timeout_secs))
            .connect_timeout(Duration::from_secs(connect_timeout_secs))
            .redirect(Policy::limited(MAX_REDIRECTS))
            .user_agent(config.user_agent.clone())
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        Self { config, client }
    }

    fn validate_url(url: &str) -> Result<url::Url> {
        let parsed = url::Url::parse(url).map_err(|e| anyhow::anyhow!("Invalid URL: {e}"))?;
        match parsed.scheme() {
            "http" | "https" => {}
            other => anyhow::bail!("Unsupported URL scheme '{other}', only http/https are allowed"),
        }
        if parsed.host_str().is_none() {
            anyhow::bail!("URL must include a host");
        }
        Ok(parsed)
    }

    fn detect_html(content_type: &str, body: &str) -> bool {
        if content_type.contains("text/html") {
            return true;
        }
        let probe = body.trim_start().to_ascii_lowercase();
        probe.starts_with("<!doctype html") || probe.starts_with("<html")
    }

    fn clamp_max_chars(&self, requested: Option<u64>) -> usize {
        match requested {
            Some(v) => usize::try_from(v)
                .unwrap_or(self.config.max_response_size)
                .max(1)
                .min(self.config.max_response_size),
            None => self.config.max_response_size,
        }
    }

    fn truncate_chars(text: &str, max_chars: usize) -> (String, bool) {
        let total_chars = text.chars().count();
        if total_chars <= max_chars {
            return (text.to_string(), false);
        }
        let truncated = text.chars().take(max_chars).collect::<String>();
        (truncated, true)
    }

    fn normalize_whitespace(text: &str) -> String {
        let mut out = text.lines().map(str::trim).collect::<Vec<_>>().join("\n");
        while out.contains("\n\n\n") {
            out = out.replace("\n\n\n", "\n\n");
        }
        out.trim().to_string()
    }

    fn html_to_text(html: &str) -> String {
        static SCRIPT_RE: OnceLock<Regex> = OnceLock::new();
        static STYLE_RE: OnceLock<Regex> = OnceLock::new();
        static BR_RE: OnceLock<Regex> = OnceLock::new();
        static BLOCK_CLOSE_RE: OnceLock<Regex> = OnceLock::new();
        static TAG_RE: OnceLock<Regex> = OnceLock::new();

        let script_re =
            SCRIPT_RE.get_or_init(|| Regex::new(r"(?is)<script[^>]*>.*?</script>").unwrap());
        let style_re =
            STYLE_RE.get_or_init(|| Regex::new(r"(?is)<style[^>]*>.*?</style>").unwrap());
        let br_re = BR_RE.get_or_init(|| Regex::new(r"(?is)<br\s*/?>").unwrap());
        let block_close_re = BLOCK_CLOSE_RE
            .get_or_init(|| Regex::new(r"(?is)</(p|div|section|article|li|h[1-6])>").unwrap());
        let tag_re = TAG_RE.get_or_init(|| Regex::new(r"(?is)<[^>]+>").unwrap());

        let no_script = script_re.replace_all(html, " ");
        let no_style = style_re.replace_all(&no_script, " ");
        let with_breaks = br_re.replace_all(&no_style, "\n");
        let with_blocks = block_close_re.replace_all(&with_breaks, "\n");
        let stripped = tag_re.replace_all(&with_blocks, " ");
        Self::normalize_whitespace(&stripped)
    }

    fn html_to_markdown_like(html: &str) -> String {
        static LINK_RE: OnceLock<Regex> = OnceLock::new();
        let link_re = LINK_RE.get_or_init(|| {
            Regex::new(r#"(?is)<a\s+[^>]*href=["']([^"']+)["'][^>]*>(.*?)</a>"#).unwrap()
        });
        let with_links = link_re.replace_all(html, "[$2]($1)");
        Self::html_to_text(&with_links)
    }

    fn should_retry_error(err: &reqwest::Error) -> bool {
        err.is_timeout() || err.is_connect() || err.is_request()
    }

    fn should_retry_status(status: StatusCode) -> bool {
        status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error()
    }
}

#[async_trait]
impl Tool for WebFetchTool {
    fn name(&self) -> &str {
        "web_fetch"
    }

    fn description(&self) -> &str {
        "Fetch a URL and extract readable text/markdown content."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "url": {
                    "type": "string",
                    "description": "HTTP(S) URL to fetch."
                },
                "extract_mode": {
                    "type": "string",
                    "enum": ["text", "markdown"],
                    "description": "Content extraction mode. Defaults to text."
                },
                "max_chars": {
                    "type": "integer",
                    "description": "Maximum output characters (clamped to configured tool limit)."
                }
            },
            "required": ["url"]
        })
    }

    async fn execute(&self, args: serde_json::Value) -> Result<ToolResult> {
        let url = args
            .get("url")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing required argument: url"))?;

        let extract_mode = args
            .get("extract_mode")
            .and_then(|v| v.as_str())
            .unwrap_or("text")
            .to_ascii_lowercase();
        if extract_mode != "text" && extract_mode != "markdown" {
            return Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some("extract_mode must be 'text' or 'markdown'".to_string()),
            });
        }

        let max_chars = self.clamp_max_chars(args.get("max_chars").and_then(|v| v.as_u64()));
        if let Err(err) = Self::validate_url(url) {
            return Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some(err.to_string()),
            });
        }

        let mut response = None;
        for attempt in 1..=MAX_REQUEST_ATTEMPTS {
            let sent = self.client.get(url).send().await;
            let candidate = match sent {
                Ok(resp) => resp,
                Err(err) => {
                    if attempt < MAX_REQUEST_ATTEMPTS && Self::should_retry_error(&err) {
                        tokio::time::sleep(Duration::from_millis(RETRY_DELAY_MS)).await;
                        continue;
                    }
                    return Ok(ToolResult {
                        success: false,
                        output: String::new(),
                        error: Some(format!("web_fetch request failed: {err}")),
                    });
                }
            };

            if attempt < MAX_REQUEST_ATTEMPTS && Self::should_retry_status(candidate.status()) {
                tokio::time::sleep(Duration::from_millis(RETRY_DELAY_MS)).await;
                continue;
            }

            response = Some(candidate);
            break;
        }

        let Some(response) = response else {
            return Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some("web_fetch failed after retry".to_string()),
            });
        };

        let status = response.status();
        let final_url = response.url().to_string();
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_ascii_lowercase();

        let body = match response.text().await {
            Ok(text) => text,
            Err(err) => {
                return Ok(ToolResult {
                    success: false,
                    output: String::new(),
                    error: Some(format!("web_fetch failed to read response body: {err}")),
                });
            }
        };

        if !status.is_success() {
            let (snippet, truncated) = Self::truncate_chars(body.trim(), 400);
            return Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some(format!(
                    "web_fetch HTTP {} for '{}'. body{}: {}",
                    status,
                    url,
                    if truncated { " (truncated)" } else { "" },
                    snippet
                )),
            });
        }

        let extracted = if Self::detect_html(&content_type, &body) {
            if extract_mode == "markdown" {
                Self::html_to_markdown_like(&body)
            } else {
                Self::html_to_text(&body)
            }
        } else if content_type.contains("text/plain")
            || content_type.contains("text/markdown")
            || content_type.contains("application/json")
            || content_type.is_empty()
        {
            body
        } else {
            return Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some(format!(
                    "Unsupported content type: '{}'. Supported: text/html, text/plain, text/markdown, application/json",
                    content_type
                )),
            });
        };

        let (text, truncated) = Self::truncate_chars(&extracted, max_chars);
        let output = format!(
            "URL: {url}\nFinal URL: {final_url}\nStatus: {status}\nContent-Type: {}\nLength: {}\nTruncated: {}\n\n{}",
            if content_type.is_empty() {
                "(unknown)"
            } else {
                content_type.as_str()
            },
            text.chars().count(),
            truncated,
            text
        );

        Ok(ToolResult {
            success: true,
            output,
            error: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::extract::State;
    use axum::http::StatusCode;
    use axum::response::Html;
    use axum::routing::get;
    use axum::{Json, Router};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use tokio::net::TcpListener;

    #[test]
    fn html_to_text_strips_script_and_style_noise() {
        let html = r#"
            <html>
              <head>
                <style>.hidden{display:none}</style>
                <script>console.log("x")</script>
              </head>
              <body>
                <h1>Title</h1>
                <p>Hello <b>World</b></p>
              </body>
            </html>
        "#;

        let text = WebFetchTool::html_to_text(html);
        assert!(text.contains("Title"));
        assert!(text.contains("Hello"));
        assert!(text.contains("World"));
        assert!(!text.contains("console.log"));
        assert!(!text.contains("hidden"));
    }

    #[test]
    fn truncate_chars_preserves_char_boundaries() {
        let input = "ab한글cd";
        let (truncated, did_truncate) = WebFetchTool::truncate_chars(input, 4);
        assert_eq!(truncated, "ab한글");
        assert!(did_truncate);
    }

    #[tokio::test]
    async fn execute_fetches_html_and_applies_max_chars() {
        let app = Router::new().route(
            "/",
            get(|| async {
                Html("<html><body><h1>Docs</h1><p>OpenClaw documentation content</p></body></html>")
            }),
        );
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        let tool = WebFetchTool::new(WebFetchConfig {
            enabled: true,
            timeout_secs: 10,
            max_response_size: 16,
            user_agent: "rikabot-test".to_string(),
        });

        let result = tool
            .execute(json!({
                "url": format!("http://{addr}/"),
                "extract_mode": "text"
            }))
            .await
            .unwrap();

        server.abort();

        assert!(result.success, "expected success, got {:?}", result.error);
        assert!(result.output.contains("Status: 200 OK"));
        assert!(result.output.contains("Truncated: true"));
        assert!(result.output.contains("Docs"));
    }

    #[tokio::test]
    async fn execute_rejects_non_http_scheme() {
        let tool = WebFetchTool::new(WebFetchConfig::default());
        let result = tool
            .execute(json!({
                "url": "file:///etc/passwd"
            }))
            .await
            .unwrap();

        assert!(!result.success);
        assert!(result
            .error
            .unwrap_or_default()
            .contains("Unsupported URL scheme"));
    }

    #[tokio::test]
    async fn execute_retries_after_transient_503() {
        let attempts = Arc::new(AtomicUsize::new(0));
        let app_state = attempts.clone();
        let app = Router::new().route(
            "/",
            get(|State(state): State<Arc<AtomicUsize>>| async move {
                let attempt = state.fetch_add(1, Ordering::SeqCst);
                if attempt == 0 {
                    return (
                        StatusCode::SERVICE_UNAVAILABLE,
                        Json(json!({"error":"temporary"})),
                    );
                }
                (
                    StatusCode::OK,
                    Json(json!({"title":"OpenClaw Docs","body":"Reliable response"})),
                )
            }),
        );
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            axum::serve(listener, app.with_state(app_state))
                .await
                .unwrap();
        });

        let tool = WebFetchTool::new(WebFetchConfig {
            enabled: true,
            timeout_secs: 10,
            max_response_size: 2000,
            user_agent: "rikabot-test".to_string(),
        });

        let result = tool
            .execute(json!({
                "url": format!("http://{addr}/"),
                "extract_mode": "text"
            }))
            .await
            .unwrap();

        server.abort();

        assert!(result.success, "expected success, got {:?}", result.error);
        assert!(result.output.contains("Status: 200 OK"));
        assert!(result.output.contains("OpenClaw Docs"));
        assert!(attempts.load(Ordering::SeqCst) >= 2);
    }
}

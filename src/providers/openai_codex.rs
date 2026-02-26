use super::*;
use anyhow::Result;

pub struct OpenAiCodexProvider {
    #[allow(dead_code)]
    oauth_token: String,
    #[allow(dead_code)]
    account_id: String,
}

impl OpenAiCodexProvider {
    pub fn new(oauth_token: &str, account_id: &str) -> Self {
        Self {
            oauth_token: oauth_token.to_string(),
            account_id: account_id.to_string(),
        }
    }
}

#[async_trait]
impl Provider for OpenAiCodexProvider {
    fn supports_native_tools(&self) -> bool {
        false
    }

    async fn chat(
        &self,
        _messages: &[ChatMessage],
        _tools: Option<&[ToolSpec]>,
        _model: &str,
        _temperature: f64,
    ) -> Result<ChatResponse> {
        anyhow::bail!("OpenAI Codex provider not yet implemented")
    }
}

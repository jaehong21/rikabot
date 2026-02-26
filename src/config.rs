use anyhow::Result;
use serde::Deserialize;
use std::path::PathBuf;

#[derive(Debug, Clone, Deserialize)]
pub struct AppConfig {
    #[serde(default = "default_host")]
    pub host: String,
    #[serde(default = "default_port")]
    pub port: u16,
    #[serde(default = "default_system_prompt")]
    pub system_prompt: String,
    pub provider: ProviderConfig,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ProviderConfig {
    pub kind: String, // "openrouter" | "openai_codex"
    #[serde(default = "default_model")]
    pub model: String,
    #[serde(default = "default_temperature")]
    pub temperature: f64,
    pub api_key: Option<String>,
    pub oauth_token: Option<String>,
    pub account_id: Option<String>,
}

fn default_host() -> String {
    "127.0.0.1".to_string()
}
fn default_port() -> u16 {
    4728
}
fn default_system_prompt() -> String {
    "You are Rika, a helpful personal AI assistant. You can execute shell commands using the shell tool when needed.".to_string()
}
fn default_model() -> String {
    "openai/gpt-5.2".to_string()
}
fn default_temperature() -> f64 {
    0.1
}

impl AppConfig {
    pub fn load(path: Option<&str>) -> Result<Self> {
        let config_path = path
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("config.toml"));

        let contents = std::fs::read_to_string(&config_path)
            .map_err(|e| anyhow::anyhow!("Failed to read config file {:?}: {}", config_path, e))?;

        let mut config: AppConfig = toml::from_str(&contents)
            .map_err(|e| anyhow::anyhow!("Failed to parse config: {}", e))?;

        // Environment variable overrides
        if let Ok(key) = std::env::var("RIKABOT_PROVIDER_API_KEY") {
            config.provider.api_key = Some(key);
        }
        if let Ok(token) = std::env::var("RIKABOT_PROVIDER_OAUTH_TOKEN") {
            config.provider.oauth_token = Some(token);
        }
        if let Ok(id) = std::env::var("RIKABOT_PROVIDER_ACCOUNT_ID") {
            config.provider.account_id = Some(id);
        }

        Ok(config)
    }
}

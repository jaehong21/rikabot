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
    #[serde(default = "default_model")]
    pub model: String,
    #[serde(default = "default_temperature")]
    pub temperature: f64,
    pub provider: String,
    pub providers: ProvidersConfig,
    /// Optional path to workspace directory. Defaults to ~/.rika/workspace
    #[serde(default)]
    pub workspace_dir: Option<String>,
    #[serde(default)]
    pub skills: SkillsConfig,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SkillsConfig {
    #[serde(default = "default_skills_enabled")]
    pub enabled: bool,
}

impl Default for SkillsConfig {
    fn default() -> Self {
        Self { enabled: true }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct ProvidersConfig {
    pub openrouter: Option<OpenRouterConfig>,
    pub openai_codex: Option<OpenAiCodexConfig>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct OpenRouterConfig {
    pub api_key: Option<String>,
    pub env_key: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct OpenAiCodexConfig {
    pub oauth_token: Option<String>,
    pub env_key: Option<String>,
    pub account_id: Option<String>,
    pub env_account_id: Option<String>,
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
fn default_skills_enabled() -> bool {
    true
}

impl OpenRouterConfig {
    pub fn resolve_api_key(&self) -> Result<String> {
        if let Some(key) = &self.api_key {
            return Ok(key.clone());
        }
        if let Some(env) = &self.env_key {
            return std::env::var(env)
                .map_err(|_| anyhow::anyhow!("env var '{}' not set for openrouter api_key", env));
        }
        anyhow::bail!("openrouter requires api_key or env_key")
    }
}

impl OpenAiCodexConfig {
    pub fn resolve_oauth_token(&self) -> Result<String> {
        if let Some(token) = &self.oauth_token {
            return Ok(token.clone());
        }
        if let Some(env) = &self.env_key {
            return std::env::var(env).map_err(|_| {
                anyhow::anyhow!("env var '{}' not set for openai_codex oauth_token", env)
            });
        }
        anyhow::bail!("openai_codex requires oauth_token or env_key")
    }

    pub fn resolve_account_id(&self) -> Result<String> {
        if let Some(id) = &self.account_id {
            return Ok(id.clone());
        }
        if let Some(env) = &self.env_account_id {
            return std::env::var(env).map_err(|_| {
                anyhow::anyhow!("env var '{}' not set for openai_codex account_id", env)
            });
        }
        anyhow::bail!("openai_codex requires account_id or env_account_id")
    }
}

impl AppConfig {
    pub fn load(path: Option<&str>) -> Result<Self> {
        let config_path = path
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("config.toml"));

        let contents = std::fs::read_to_string(&config_path)
            .map_err(|e| anyhow::anyhow!("Failed to read config file {:?}: {}", config_path, e))?;

        let config: AppConfig = toml::from_str(&contents)
            .map_err(|e| anyhow::anyhow!("Failed to parse config: {}", e))?;

        Ok(config)
    }
}

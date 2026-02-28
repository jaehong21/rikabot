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
    #[serde(default)]
    pub prompt: PromptConfig,
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
pub struct PromptConfig {
    #[serde(default = "default_bootstrap_max_chars")]
    pub bootstrap_max_chars: usize,
    #[serde(default = "default_bootstrap_total_max_chars")]
    pub bootstrap_total_max_chars: usize,
}

impl Default for PromptConfig {
    fn default() -> Self {
        Self {
            bootstrap_max_chars: default_bootstrap_max_chars(),
            bootstrap_total_max_chars: default_bootstrap_total_max_chars(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct ProvidersConfig {
    pub openai: Option<OpenAiConfig>,
    pub openrouter: Option<OpenRouterConfig>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct OpenAiConfig {
    pub api_key: Option<String>,
    pub env_key: Option<String>,
    pub base_url: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct OpenRouterConfig {
    pub api_key: Option<String>,
    pub env_key: Option<String>,
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
fn default_bootstrap_max_chars() -> usize {
    20_000
}
fn default_bootstrap_total_max_chars() -> usize {
    150_000
}

impl OpenAiConfig {
    pub fn resolve_api_key(&self) -> Result<String> {
        if let Some(key) = &self.api_key {
            return Ok(key.clone());
        }
        if let Some(env) = &self.env_key {
            return std::env::var(env)
                .map_err(|_| anyhow::anyhow!("env var '{}' not set for openai api_key", env));
        }
        anyhow::bail!("openai requires api_key or env_key")
    }

    pub fn resolve_base_url(&self) -> Result<String> {
        const DEFAULT_OPENAI_BASE_URL: &str = "https://api.openai.com/v1";

        let raw = self
            .base_url
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or(DEFAULT_OPENAI_BASE_URL);

        let parsed = reqwest::Url::parse(raw)
            .map_err(|_| anyhow::anyhow!("openai base_url must be a valid URL"))?;

        match parsed.scheme() {
            "http" | "https" => {}
            _ => anyhow::bail!("openai base_url must use http or https"),
        }

        let path = parsed.path().trim_end_matches('/');
        if path.ends_with("/chat/completions") {
            anyhow::bail!("openai base_url must be a base URL without '/chat/completions' suffix");
        }

        Ok(raw.trim_end_matches('/').to_string())
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn openai_resolve_api_key_from_config() {
        let cfg = OpenAiConfig {
            api_key: Some("test-key".to_string()),
            env_key: None,
            base_url: None,
        };
        assert_eq!(cfg.resolve_api_key().unwrap(), "test-key");
    }

    #[test]
    fn openai_resolve_api_key_from_env_key() {
        let env_name = "RIKABOT_TEST_OPENAI_API_KEY";
        unsafe { std::env::set_var(env_name, "env-test-key") };

        let cfg = OpenAiConfig {
            api_key: None,
            env_key: Some(env_name.to_string()),
            base_url: None,
        };
        assert_eq!(cfg.resolve_api_key().unwrap(), "env-test-key");
        unsafe { std::env::remove_var(env_name) };
    }

    #[test]
    fn openai_resolve_base_url_uses_default() {
        let cfg = OpenAiConfig {
            api_key: None,
            env_key: None,
            base_url: None,
        };
        assert_eq!(cfg.resolve_base_url().unwrap(), "https://api.openai.com/v1");
    }

    #[test]
    fn openai_resolve_base_url_trims_trailing_slash() {
        let cfg = OpenAiConfig {
            api_key: None,
            env_key: None,
            base_url: Some("https://gateway.example.com/v1/".to_string()),
        };
        assert_eq!(
            cfg.resolve_base_url().unwrap(),
            "https://gateway.example.com/v1"
        );
    }

    #[test]
    fn openai_resolve_base_url_rejects_chat_completions_suffix() {
        let cfg = OpenAiConfig {
            api_key: None,
            env_key: None,
            base_url: Some("https://api.openai.com/v1/chat/completions".to_string()),
        };
        let err = cfg.resolve_base_url().unwrap_err().to_string();
        assert!(err.contains("base URL without '/chat/completions'"));
    }
}

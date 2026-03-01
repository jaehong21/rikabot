use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

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
    #[serde(default)]
    pub permissions: PermissionsConfig,
    #[serde(default)]
    pub mcp: McpConfig,
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

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PermissionsConfig {
    #[serde(default = "default_permissions_enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub tools: ToolPermissionsConfig,
}

impl Default for PermissionsConfig {
    fn default() -> Self {
        Self {
            enabled: default_permissions_enabled(),
            tools: ToolPermissionsConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct ToolPermissionsConfig {
    #[serde(default)]
    pub allow: Vec<String>,
    #[serde(default)]
    pub deny: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct McpConfig {
    #[serde(default = "default_mcp_enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub servers: Vec<McpServerConfig>,
}

impl Default for McpConfig {
    fn default() -> Self {
        Self {
            enabled: default_mcp_enabled(),
            servers: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum McpTransport {
    Stdio,
    Http,
}

impl Default for McpTransport {
    fn default() -> Self {
        Self::Stdio
    }
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum McpAuthMode {
    Headers,
    Oauth,
}

impl Default for McpAuthMode {
    fn default() -> Self {
        Self::Headers
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct McpServerConfig {
    pub name: String,
    #[serde(default = "default_mcp_server_enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub transport: McpTransport,
    #[serde(default)]
    pub auth_mode: McpAuthMode,
    pub command: Option<String>,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: HashMap<String, String>,
    pub cwd: Option<String>,
    pub url: Option<String>,
    #[serde(default)]
    pub headers: HashMap<String, String>,
    pub oauth_client_id: Option<String>,
    pub oauth_client_secret_env: Option<String>,
    #[serde(default)]
    pub oauth_scopes: Vec<String>,
    pub oauth_authorization_server: Option<String>,
    pub tool_timeout_secs: Option<u64>,
    pub init_timeout_secs: Option<u64>,
}

impl McpServerConfig {
    pub fn resolved_tool_timeout_secs(&self) -> u64 {
        self.tool_timeout_secs
            .unwrap_or(default_mcp_tool_timeout_secs())
            .min(max_mcp_tool_timeout_secs())
    }

    pub fn resolved_init_timeout_secs(&self) -> u64 {
        self.init_timeout_secs
            .unwrap_or(default_mcp_init_timeout_secs())
    }

    pub fn resolved_http_headers(&self) -> Result<HashMap<String, String>> {
        let mut resolved = HashMap::with_capacity(self.headers.len());
        for (key, value) in &self.headers {
            resolved.insert(key.clone(), resolve_env_placeholders(value)?);
        }
        Ok(resolved)
    }

    pub fn resolved_oauth_client_secret(&self) -> Result<Option<String>> {
        let Some(env_name) = self.oauth_client_secret_env.as_deref() else {
            return Ok(None);
        };
        let env_name = env_name.trim();
        if env_name.is_empty() {
            anyhow::bail!("oauth_client_secret_env cannot be empty");
        }
        let secret = std::env::var(env_name).map_err(|_| {
            anyhow::anyhow!("env var '{}' not set for MCP OAuth client secret", env_name)
        })?;
        Ok(Some(secret))
    }
}

fn resolve_env_placeholders(input: &str) -> Result<String> {
    if input.contains("${") && !input.contains('}') {
        anyhow::bail!("unterminated env placeholder in header value");
    }

    let re = regex::Regex::new(r"\$\{([A-Za-z_][A-Za-z0-9_]*)\}")
        .map_err(|e| anyhow::anyhow!("failed to compile env placeholder regex: {}", e))?;

    let mut out = String::with_capacity(input.len());
    let mut last = 0usize;
    for caps in re.captures_iter(input) {
        let full = caps.get(0).expect("capture 0 should exist");
        let var = caps
            .get(1)
            .expect("capture 1 should exist for env placeholder")
            .as_str();
        out.push_str(&input[last..full.start()]);
        let env_val = std::env::var(var)
            .map_err(|_| anyhow::anyhow!("env var '{}' not set for MCP header", var))?;
        out.push_str(&env_val);
        last = full.end();
    }
    out.push_str(&input[last..]);
    if out.contains("${") {
        anyhow::bail!("invalid env placeholder format in header value");
    }
    Ok(out)
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
fn default_mcp_enabled() -> bool {
    true
}
fn default_permissions_enabled() -> bool {
    true
}
fn default_mcp_server_enabled() -> bool {
    true
}
fn default_mcp_tool_timeout_secs() -> u64 {
    180
}
fn max_mcp_tool_timeout_secs() -> u64 {
    600
}
fn default_mcp_init_timeout_secs() -> u64 {
    30
}

impl McpConfig {
    pub fn validate(&self) -> Result<()> {
        let mut names = HashSet::new();

        for server in &self.servers {
            if !names.insert(server.name.clone()) {
                anyhow::bail!("duplicate MCP server name '{}'", server.name);
            }
            if server.name.trim().is_empty() {
                anyhow::bail!("MCP server name cannot be empty");
            }

            match server.transport {
                McpTransport::Stdio => {
                    let command = server.command.as_deref().map(str::trim).unwrap_or("");
                    if command.is_empty() {
                        anyhow::bail!(
                            "MCP server '{}' with stdio transport requires non-empty command",
                            server.name
                        );
                    }
                }
                McpTransport::Http => {
                    let url = server.url.as_deref().map(str::trim).unwrap_or("");
                    if url.is_empty() {
                        anyhow::bail!(
                            "MCP server '{}' with http transport requires non-empty url",
                            server.name
                        );
                    }
                    let parsed = reqwest::Url::parse(url).map_err(|_| {
                        anyhow::anyhow!("MCP server '{}' has invalid url", server.name)
                    })?;
                    match parsed.scheme() {
                        "http" | "https" => {}
                        _ => {
                            anyhow::bail!("MCP server '{}' url must use http or https", server.name)
                        }
                    }
                }
            }

            match server.auth_mode {
                McpAuthMode::Headers => {}
                McpAuthMode::Oauth => {
                    if server.transport != McpTransport::Http {
                        anyhow::bail!(
                            "MCP server '{}' with oauth auth_mode requires http transport",
                            server.name
                        );
                    }

                    if server
                        .headers
                        .keys()
                        .any(|k| k.eq_ignore_ascii_case("authorization"))
                    {
                        anyhow::bail!(
                            "MCP server '{}' with oauth auth_mode must not set Authorization header",
                            server.name
                        );
                    }

                    if let Some(raw) = server.oauth_authorization_server.as_deref() {
                        let value = raw.trim();
                        if value.is_empty() {
                            anyhow::bail!(
                                "MCP server '{}' oauth_authorization_server cannot be empty",
                                server.name
                            );
                        }
                        let parsed = reqwest::Url::parse(value).map_err(|_| {
                            anyhow::anyhow!(
                                "MCP server '{}' has invalid oauth_authorization_server url",
                                server.name
                            )
                        })?;
                        match parsed.scheme() {
                            "http" | "https" => {}
                            _ => anyhow::bail!(
                                "MCP server '{}' oauth_authorization_server must use http or https",
                                server.name
                            ),
                        }
                    }
                }
            }
        }

        Ok(())
    }
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
    pub fn resolve_path(path: Option<&str>) -> Result<PathBuf> {
        if let Some(raw) = path {
            return resolve_path_str(raw);
        }

        if let Ok(raw) = std::env::var("RIKA_CONFIG") {
            return resolve_path_str(&raw);
        }

        let home = resolve_home_dir().ok_or_else(|| {
            anyhow::anyhow!("could not resolve home directory for default config path")
        })?;
        Ok(home.join(".rika").join("config.toml"))
    }

    pub fn load_from_path(config_path: &Path) -> Result<Self> {
        let contents = std::fs::read_to_string(config_path)
            .map_err(|e| anyhow::anyhow!("Failed to read config file {:?}: {}", config_path, e))?;

        let config: AppConfig = toml::from_str(&contents)
            .map_err(|e| anyhow::anyhow!("Failed to parse config: {}", e))?;
        config.mcp.validate()?;

        Ok(config)
    }

    pub fn load(path: Option<&str>) -> Result<Self> {
        let config_path = Self::resolve_path(path)?;
        Self::load_from_path(&config_path)
    }
}

fn resolve_path_str(raw: &str) -> Result<PathBuf> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        anyhow::bail!("config path cannot be empty");
    }

    if trimmed == "~" {
        let home = resolve_home_dir()
            .ok_or_else(|| anyhow::anyhow!("could not resolve home directory for config path"))?;
        return Ok(home);
    }

    if let Some(rest) = trimmed.strip_prefix("~/") {
        let home = resolve_home_dir()
            .ok_or_else(|| anyhow::anyhow!("could not resolve home directory for config path"))?;
        return Ok(home.join(rest));
    }

    Ok(PathBuf::from(trimmed))
}

fn resolve_home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("USERPROFILE").map(PathBuf::from))
        .or_else(|| {
            let drive = std::env::var_os("HOMEDRIVE")?;
            let path = std::env::var_os("HOMEPATH")?;
            Some(PathBuf::from(drive).join(path))
        })
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

    #[test]
    fn mcp_default_enabled_and_empty_servers() {
        let cfg = McpConfig::default();
        assert!(cfg.enabled);
        assert!(cfg.servers.is_empty());
    }

    #[test]
    fn mcp_transport_default_is_stdio() {
        let server = McpServerConfig {
            name: "s".to_string(),
            enabled: true,
            transport: McpTransport::default(),
            auth_mode: McpAuthMode::default(),
            command: Some("cmd".to_string()),
            args: vec![],
            env: HashMap::new(),
            cwd: None,
            url: None,
            headers: HashMap::new(),
            oauth_client_id: None,
            oauth_client_secret_env: None,
            oauth_scopes: vec![],
            oauth_authorization_server: None,
            tool_timeout_secs: None,
            init_timeout_secs: None,
        };
        assert_eq!(server.transport, McpTransport::Stdio);
    }

    #[test]
    fn mcp_validate_rejects_duplicate_names() {
        let cfg = McpConfig {
            enabled: true,
            servers: vec![
                McpServerConfig {
                    name: "dup".to_string(),
                    enabled: true,
                    transport: McpTransport::Stdio,
                    auth_mode: McpAuthMode::Headers,
                    command: Some("echo".to_string()),
                    args: vec![],
                    env: HashMap::new(),
                    cwd: None,
                    url: None,
                    headers: HashMap::new(),
                    oauth_client_id: None,
                    oauth_client_secret_env: None,
                    oauth_scopes: vec![],
                    oauth_authorization_server: None,
                    tool_timeout_secs: None,
                    init_timeout_secs: None,
                },
                McpServerConfig {
                    name: "dup".to_string(),
                    enabled: true,
                    transport: McpTransport::Http,
                    auth_mode: McpAuthMode::Headers,
                    command: None,
                    args: vec![],
                    env: HashMap::new(),
                    cwd: None,
                    url: Some("https://example.com/mcp".to_string()),
                    headers: HashMap::new(),
                    oauth_client_id: None,
                    oauth_client_secret_env: None,
                    oauth_scopes: vec![],
                    oauth_authorization_server: None,
                    tool_timeout_secs: None,
                    init_timeout_secs: None,
                },
            ],
        };
        let err = cfg.validate().unwrap_err().to_string();
        assert!(err.contains("duplicate MCP server name"));
    }

    #[test]
    fn mcp_validate_rejects_missing_stdio_command() {
        let cfg = McpConfig {
            enabled: true,
            servers: vec![McpServerConfig {
                name: "stdio".to_string(),
                enabled: true,
                transport: McpTransport::Stdio,
                auth_mode: McpAuthMode::Headers,
                command: None,
                args: vec![],
                env: HashMap::new(),
                cwd: None,
                url: None,
                headers: HashMap::new(),
                oauth_client_id: None,
                oauth_client_secret_env: None,
                oauth_scopes: vec![],
                oauth_authorization_server: None,
                tool_timeout_secs: None,
                init_timeout_secs: None,
            }],
        };
        let err = cfg.validate().unwrap_err().to_string();
        assert!(err.contains("requires non-empty command"));
    }

    #[test]
    fn mcp_validate_rejects_missing_http_url() {
        let cfg = McpConfig {
            enabled: true,
            servers: vec![McpServerConfig {
                name: "http".to_string(),
                enabled: true,
                transport: McpTransport::Http,
                auth_mode: McpAuthMode::Headers,
                command: None,
                args: vec![],
                env: HashMap::new(),
                cwd: None,
                url: None,
                headers: HashMap::new(),
                oauth_client_id: None,
                oauth_client_secret_env: None,
                oauth_scopes: vec![],
                oauth_authorization_server: None,
                tool_timeout_secs: None,
                init_timeout_secs: None,
            }],
        };
        let err = cfg.validate().unwrap_err().to_string();
        assert!(err.contains("requires non-empty url"));
    }

    #[test]
    fn mcp_validate_resolves_header_placeholders() {
        let env_name = "RIKABOT_TEST_MCP_TOKEN";
        unsafe { std::env::set_var(env_name, "abc123") };

        let mut headers = HashMap::new();
        headers.insert(
            "Authorization".to_string(),
            "Bearer ${RIKABOT_TEST_MCP_TOKEN}".to_string(),
        );

        let cfg = McpConfig {
            enabled: true,
            servers: vec![McpServerConfig {
                name: "http".to_string(),
                enabled: true,
                transport: McpTransport::Http,
                auth_mode: McpAuthMode::Headers,
                command: None,
                args: vec![],
                env: HashMap::new(),
                cwd: None,
                url: Some("https://mcp.linear.app/mcp".to_string()),
                headers,
                oauth_client_id: None,
                oauth_client_secret_env: None,
                oauth_scopes: vec![],
                oauth_authorization_server: None,
                tool_timeout_secs: None,
                init_timeout_secs: None,
            }],
        };
        assert!(cfg.validate().is_ok());

        unsafe { std::env::remove_var(env_name) };
    }

    #[test]
    fn mcp_header_resolution_fails_on_missing_env() {
        let mut headers = HashMap::new();
        headers.insert(
            "Authorization".to_string(),
            "Bearer ${RIKABOT_TEST_MCP_MISSING_TOKEN}".to_string(),
        );

        let server = McpServerConfig {
            name: "http".to_string(),
            enabled: true,
            transport: McpTransport::Http,
            auth_mode: McpAuthMode::Headers,
            command: None,
            args: vec![],
            env: HashMap::new(),
            cwd: None,
            url: Some("https://mcp.notion.com/mcp".to_string()),
            headers,
            oauth_client_id: None,
            oauth_client_secret_env: None,
            oauth_scopes: vec![],
            oauth_authorization_server: None,
            tool_timeout_secs: None,
            init_timeout_secs: None,
        };
        let err = server.resolved_http_headers().unwrap_err().to_string();
        assert!(err.contains("not set for MCP header"));
    }

    #[test]
    fn mcp_header_resolution_fails_on_invalid_placeholder() {
        let mut headers = HashMap::new();
        headers.insert("Authorization".to_string(), "Bearer ${BAD".to_string());

        let server = McpServerConfig {
            name: "http".to_string(),
            enabled: true,
            transport: McpTransport::Http,
            auth_mode: McpAuthMode::Headers,
            command: None,
            args: vec![],
            env: HashMap::new(),
            cwd: None,
            url: Some("https://mcp.notion.com/mcp".to_string()),
            headers,
            oauth_client_id: None,
            oauth_client_secret_env: None,
            oauth_scopes: vec![],
            oauth_authorization_server: None,
            tool_timeout_secs: None,
            init_timeout_secs: None,
        };
        let err = server.resolved_http_headers().unwrap_err().to_string();
        assert!(err.contains("unterminated env placeholder"));
    }

    #[test]
    fn mcp_oauth_mode_requires_http_transport() {
        let cfg = McpConfig {
            enabled: true,
            servers: vec![McpServerConfig {
                name: "oauth_stdio".to_string(),
                enabled: true,
                transport: McpTransport::Stdio,
                auth_mode: McpAuthMode::Oauth,
                command: Some("echo".to_string()),
                args: vec![],
                env: HashMap::new(),
                cwd: None,
                url: None,
                headers: HashMap::new(),
                oauth_client_id: None,
                oauth_client_secret_env: None,
                oauth_scopes: vec![],
                oauth_authorization_server: None,
                tool_timeout_secs: None,
                init_timeout_secs: None,
            }],
        };
        let err = cfg.validate().unwrap_err().to_string();
        assert!(err.contains("requires http transport"));
    }

    #[test]
    fn mcp_oauth_mode_rejects_authorization_header() {
        let mut headers = HashMap::new();
        headers.insert("Authorization".to_string(), "Bearer abc".to_string());

        let cfg = McpConfig {
            enabled: true,
            servers: vec![McpServerConfig {
                name: "oauth_header".to_string(),
                enabled: true,
                transport: McpTransport::Http,
                auth_mode: McpAuthMode::Oauth,
                command: None,
                args: vec![],
                env: HashMap::new(),
                cwd: None,
                url: Some("https://example.com/mcp".to_string()),
                headers,
                oauth_client_id: None,
                oauth_client_secret_env: None,
                oauth_scopes: vec![],
                oauth_authorization_server: None,
                tool_timeout_secs: None,
                init_timeout_secs: None,
            }],
        };

        let err = cfg.validate().unwrap_err().to_string();
        assert!(err.contains("must not set Authorization header"));
    }

    #[test]
    fn mcp_oauth_client_secret_resolves_from_env() {
        let env_name = "RIKABOT_TEST_MCP_OAUTH_CLIENT_SECRET";
        unsafe { std::env::set_var(env_name, "top-secret") };

        let server = McpServerConfig {
            name: "oauth".to_string(),
            enabled: true,
            transport: McpTransport::Http,
            auth_mode: McpAuthMode::Oauth,
            command: None,
            args: vec![],
            env: HashMap::new(),
            cwd: None,
            url: Some("https://example.com/mcp".to_string()),
            headers: HashMap::new(),
            oauth_client_id: None,
            oauth_client_secret_env: Some(env_name.to_string()),
            oauth_scopes: vec![],
            oauth_authorization_server: None,
            tool_timeout_secs: None,
            init_timeout_secs: None,
        };

        let resolved = server.resolved_oauth_client_secret().unwrap();
        assert_eq!(resolved.as_deref(), Some("top-secret"));
        unsafe { std::env::remove_var(env_name) };
    }

    #[test]
    fn mcp_tool_timeout_is_capped() {
        let server = McpServerConfig {
            name: "cap".to_string(),
            enabled: true,
            transport: McpTransport::Stdio,
            auth_mode: McpAuthMode::Headers,
            command: Some("echo".to_string()),
            args: vec![],
            env: HashMap::new(),
            cwd: None,
            url: None,
            headers: HashMap::new(),
            oauth_client_id: None,
            oauth_client_secret_env: None,
            oauth_scopes: vec![],
            oauth_authorization_server: None,
            tool_timeout_secs: Some(9_999),
            init_timeout_secs: None,
        };
        assert_eq!(server.resolved_tool_timeout_secs(), 600);
    }

    #[test]
    fn permissions_defaults_to_enabled_with_empty_tool_rules() {
        let cfg = PermissionsConfig::default();
        assert!(cfg.enabled);
        assert!(cfg.tools.allow.is_empty());
        assert!(cfg.tools.deny.is_empty());
    }
}

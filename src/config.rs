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
    #[serde(default = "default_model")]
    pub model: String,
    #[serde(default = "default_temperature")]
    pub temperature: f64,
    #[serde(default = "default_max_concurrent_sessions")]
    pub max_concurrent_sessions: usize,
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
    pub shell: ShellConfig,
    #[serde(default)]
    pub process: ProcessConfig,
    #[serde(default)]
    pub web_fetch: WebFetchConfig,
    #[serde(default)]
    pub web_search: WebSearchConfig,
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
pub struct ShellConfig {
    #[serde(default = "default_shell_enabled")]
    pub enabled: bool,
    #[serde(alias = "timeout_seconds")]
    #[serde(default = "default_shell_timeout_secs")]
    pub timeout_secs: u64,
    #[serde(default = "default_shell_max_output_bytes")]
    pub max_output_bytes: usize,
}

impl Default for ShellConfig {
    fn default() -> Self {
        Self {
            enabled: default_shell_enabled(),
            timeout_secs: default_shell_timeout_secs(),
            max_output_bytes: default_shell_max_output_bytes(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct ProcessConfig {
    #[serde(default = "default_process_enabled")]
    pub enabled: bool,
    #[serde(default = "default_process_max_concurrent")]
    pub max_concurrent: usize,
    #[serde(default = "default_process_max_output_bytes")]
    pub max_output_bytes: usize,
    #[serde(default = "default_process_cleanup_retention_secs")]
    pub cleanup_retention_secs: u64,
    #[serde(default = "default_process_kill_grace_secs")]
    pub kill_grace_secs: u64,
    #[serde(default = "default_process_wait_default_secs")]
    pub wait_default_secs: u64,
    #[serde(default = "default_process_wait_max_secs")]
    pub wait_max_secs: u64,
}

impl Default for ProcessConfig {
    fn default() -> Self {
        Self {
            enabled: default_process_enabled(),
            max_concurrent: default_process_max_concurrent(),
            max_output_bytes: default_process_max_output_bytes(),
            cleanup_retention_secs: default_process_cleanup_retention_secs(),
            kill_grace_secs: default_process_kill_grace_secs(),
            wait_default_secs: default_process_wait_default_secs(),
            wait_max_secs: default_process_wait_max_secs(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct WebFetchConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(alias = "timeout_seconds")]
    #[serde(default = "default_web_fetch_timeout_secs")]
    pub timeout_secs: u64,
    #[serde(default = "default_web_fetch_max_response_size")]
    pub max_response_size: usize,
    #[serde(default = "default_web_fetch_user_agent")]
    pub user_agent: String,
}

impl Default for WebFetchConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            timeout_secs: default_web_fetch_timeout_secs(),
            max_response_size: default_web_fetch_max_response_size(),
            user_agent: default_web_fetch_user_agent(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct WebSearchConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_web_search_provider")]
    pub provider: String,
    #[serde(default = "default_web_search_max_results")]
    pub max_results: usize,
    #[serde(alias = "timeout_seconds")]
    #[serde(default = "default_web_search_timeout_secs")]
    pub timeout_secs: u64,
    #[serde(default = "default_web_search_user_agent")]
    pub user_agent: String,
    #[serde(default)]
    pub providers: WebSearchProvidersConfig,
}

impl Default for WebSearchConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            provider: default_web_search_provider(),
            max_results: default_web_search_max_results(),
            timeout_secs: default_web_search_timeout_secs(),
            user_agent: default_web_search_user_agent(),
            providers: WebSearchProvidersConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct WebSearchProvidersConfig {
    #[serde(default)]
    pub openrouter: WebSearchOpenRouterConfig,
}

impl Default for WebSearchProvidersConfig {
    fn default() -> Self {
        Self {
            openrouter: WebSearchOpenRouterConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct WebSearchOpenRouterConfig {
    pub api_key: Option<String>,
    pub env_key: Option<String>,
    pub model: Option<String>,
    pub plugin_max_results: Option<usize>,
    pub plugin_search_prompt: Option<String>,
}

impl Default for WebSearchOpenRouterConfig {
    fn default() -> Self {
        Self {
            api_key: None,
            env_key: Some("OPENROUTER_API_KEY".to_string()),
            model: Some("openai/gpt-4o-mini".to_string()),
            plugin_max_results: Some(5),
            plugin_search_prompt: None,
        }
    }
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
fn default_model() -> String {
    "openai/gpt-5.2".to_string()
}
fn default_temperature() -> f64 {
    0.1
}
fn default_max_concurrent_sessions() -> usize {
    8
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
fn default_shell_enabled() -> bool {
    true
}
fn default_shell_timeout_secs() -> u64 {
    30
}
fn max_shell_timeout_secs() -> u64 {
    3_600
}
fn default_shell_max_output_bytes() -> usize {
    10_000
}
fn default_process_enabled() -> bool {
    true
}
fn default_process_max_concurrent() -> usize {
    8
}
fn max_process_max_concurrent() -> usize {
    128
}
fn default_process_max_output_bytes() -> usize {
    524_288
}
fn default_process_cleanup_retention_secs() -> u64 {
    600
}
fn default_process_kill_grace_secs() -> u64 {
    5
}
fn default_process_wait_default_secs() -> u64 {
    20
}
fn default_process_wait_max_secs() -> u64 {
    25
}
fn default_web_fetch_timeout_secs() -> u64 {
    20
}
fn default_web_fetch_max_response_size() -> usize {
    50_000
}
fn default_web_fetch_user_agent() -> String {
    "rikabot/0.1".to_string()
}
fn default_web_search_provider() -> String {
    "openrouter".to_string()
}
fn default_web_search_max_results() -> usize {
    5
}
fn default_web_search_timeout_secs() -> u64 {
    15
}
fn default_web_search_user_agent() -> String {
    "rikabot/0.1".to_string()
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WebSearchProviderKind {
    OpenRouter,
}

impl WebSearchConfig {
    pub fn resolved_provider_kind(&self) -> Result<WebSearchProviderKind> {
        match self.provider.trim().to_ascii_lowercase().as_str() {
            "openrouter" => Ok(WebSearchProviderKind::OpenRouter),
            other => anyhow::bail!(
                "web_search provider '{}' is unsupported; expected 'openrouter'",
                other
            ),
        }
    }

    pub fn resolved_max_results(&self) -> usize {
        self.max_results.clamp(1, 10)
    }
}

impl WebSearchOpenRouterConfig {
    pub fn resolve_api_key(&self) -> Result<String> {
        if let Some(key) = self
            .api_key
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
        {
            return Ok(key.to_string());
        }
        if let Some(env) = self
            .env_key
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
        {
            return std::env::var(env)
                .map_err(|_| anyhow::anyhow!("env var '{}' not set for openrouter api_key", env));
        }
        anyhow::bail!("openrouter web_search requires api_key or env_key")
    }

    pub fn resolve_model(&self) -> Result<String> {
        let model = self
            .model
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .ok_or_else(|| anyhow::anyhow!("openrouter web_search model cannot be empty"))?;
        Ok(model.to_string())
    }

    pub fn resolved_plugin_max_results(&self) -> Option<usize> {
        self.plugin_max_results.map(|v| v.clamp(1, 10))
    }

    pub fn resolved_plugin_search_prompt(&self) -> Option<String> {
        self.plugin_search_prompt
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(ToString::to_string)
    }
}

impl ShellConfig {
    pub fn resolved_timeout_secs(&self) -> u64 {
        self.timeout_secs.min(max_shell_timeout_secs())
    }
}

impl ProcessConfig {
    pub fn resolved_max_concurrent(&self) -> usize {
        self.max_concurrent.min(max_process_max_concurrent())
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
        config.validate()?;

        Ok(config)
    }

    pub fn load(path: Option<&str>) -> Result<Self> {
        let config_path = Self::resolve_path(path)?;
        Self::load_from_path(&config_path)
    }

    pub fn resolve_workspace_dir(&self) -> Result<PathBuf> {
        if let Some(raw) = self.workspace_dir.as_deref() {
            return resolve_path_str(raw);
        }

        let home = resolve_home_dir().ok_or_else(|| {
            anyhow::anyhow!("workspace_dir could not be resolved from config or HOME")
        })?;
        Ok(home.join(".rika").join("workspace"))
    }

    pub fn validate(&self) -> Result<()> {
        if self.max_concurrent_sessions == 0 {
            anyhow::bail!("max_concurrent_sessions must be greater than 0");
        }
        if self.shell.timeout_secs == 0 {
            anyhow::bail!("shell timeout_secs must be greater than 0");
        }
        if self.shell.max_output_bytes == 0 {
            anyhow::bail!("shell max_output_bytes must be greater than 0");
        }
        if self.process.max_concurrent == 0 {
            anyhow::bail!("process max_concurrent must be greater than 0");
        }
        if self.process.max_output_bytes == 0 {
            anyhow::bail!("process max_output_bytes must be greater than 0");
        }
        if self.process.cleanup_retention_secs == 0 {
            anyhow::bail!("process cleanup_retention_secs must be greater than 0");
        }
        if self.process.kill_grace_secs == 0 {
            anyhow::bail!("process kill_grace_secs must be greater than 0");
        }
        if self.process.wait_default_secs == 0 {
            anyhow::bail!("process wait_default_secs must be greater than 0");
        }
        if self.process.wait_max_secs == 0 {
            anyhow::bail!("process wait_max_secs must be greater than 0");
        }
        if self.process.wait_default_secs > self.process.wait_max_secs {
            anyhow::bail!("process wait_default_secs must be less than or equal to wait_max_secs");
        }

        if self.web_fetch.timeout_secs == 0 {
            anyhow::bail!("web_fetch timeout_secs must be greater than 0");
        }
        if self.web_fetch.max_response_size == 0 {
            anyhow::bail!("web_fetch max_response_size must be greater than 0");
        }
        if self.web_search.timeout_secs == 0 {
            anyhow::bail!("web_search timeout_secs must be greater than 0");
        }
        let _ = self.web_search.resolved_provider_kind()?;

        if self.web_search.enabled {
            let _ = self.web_search.providers.openrouter.resolve_api_key()?;
            let _ = self.web_search.providers.openrouter.resolve_model()?;
        }

        Ok(())
    }

    pub fn resolved_max_concurrent_sessions(&self) -> usize {
        self.max_concurrent_sessions
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
    fn resolve_path_str_expands_tilde_prefix() {
        let Some(home) = resolve_home_dir() else {
            return;
        };
        let path = resolve_path_str("~/test/path").expect("resolve path");
        assert_eq!(path, home.join("test").join("path"));
    }

    #[test]
    fn workspace_dir_expands_tilde() {
        let Some(home) = resolve_home_dir() else {
            return;
        };

        let cfg: AppConfig = toml::from_str(
            r#"
provider = "openai"
workspace_dir = "~/.rika/workspace"
[providers.openai]
api_key = "x"
"#,
        )
        .expect("parse config");

        let workspace = cfg.resolve_workspace_dir().expect("resolve workspace");
        assert_eq!(workspace, home.join(".rika").join("workspace"));
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

    #[test]
    fn web_configs_default_values_from_minimal_toml() {
        let cfg: AppConfig = toml::from_str(
            r#"
provider = "openai"
[providers.openai]
api_key = "x"
"#,
        )
        .expect("parse config");

        assert!(!cfg.web_fetch.enabled);
        assert_eq!(cfg.web_fetch.timeout_secs, 20);
        assert_eq!(cfg.web_fetch.max_response_size, 50_000);
        assert_eq!(cfg.web_fetch.user_agent, "rikabot/0.1");

        assert!(!cfg.web_search.enabled);
        assert_eq!(cfg.web_search.provider, "openrouter");
        assert_eq!(cfg.web_search.max_results, 5);
        assert_eq!(cfg.web_search.timeout_secs, 15);
        assert_eq!(cfg.web_search.user_agent, "rikabot/0.1");
        assert_eq!(
            cfg.web_search.providers.openrouter.model.as_deref(),
            Some("openai/gpt-4o-mini")
        );
    }

    #[test]
    fn web_search_validate_rejects_unknown_provider() {
        let cfg: AppConfig = toml::from_str(
            r#"
provider = "openai"
[providers.openai]
api_key = "x"

[web_search]
enabled = true
provider = "unknown"
"#,
        )
        .expect("parse config");

        let err = cfg.validate().unwrap_err().to_string();
        assert!(err.contains("web_search provider"));
        assert!(err.contains("unsupported"));
    }

    #[test]
    fn web_search_validate_rejects_perplexity_provider() {
        let cfg: AppConfig = toml::from_str(
            r#"
provider = "openai"
[providers.openai]
api_key = "x"

[web_search]
enabled = true
provider = "perplexity"
"#,
        )
        .expect("parse config");

        let err = cfg.validate().unwrap_err().to_string();
        assert!(err.contains("expected 'openrouter'"));
    }

    #[test]
    fn web_search_validate_uses_env_key_for_openrouter() {
        let env_name = "RIKABOT_TEST_WEB_SEARCH_OPENROUTER_KEY";
        unsafe { std::env::set_var(env_name, "or-key") };

        let cfg: AppConfig = toml::from_str(&format!(
            r#"
provider = "openai"
[providers.openai]
api_key = "x"

[web_search]
enabled = true
provider = "openrouter"

[web_search.providers.openrouter]
env_key = "{env_name}"
model = "openai/gpt-4o-mini"
"#
        ))
        .expect("parse config");

        assert!(cfg.validate().is_ok());
        unsafe { std::env::remove_var(env_name) };
    }

    #[test]
    fn web_search_resolved_max_results_clamps_to_range() {
        let mut cfg = WebSearchConfig::default();
        cfg.max_results = 0;
        assert_eq!(cfg.resolved_max_results(), 1);
        cfg.max_results = 42;
        assert_eq!(cfg.resolved_max_results(), 10);
    }

    #[test]
    fn web_search_accepts_timeout_seconds_alias() {
        let cfg: AppConfig = toml::from_str(
            r#"
provider = "openai"
[providers.openai]
api_key = "x"

[web_search]
enabled = false
provider = "openrouter"
timeout_seconds = 20
"#,
        )
        .expect("parse config");

        assert_eq!(cfg.web_search.timeout_secs, 20);
    }

    #[test]
    fn shell_and_process_defaults_from_minimal_toml() {
        let cfg: AppConfig = toml::from_str(
            r#"
provider = "openai"
[providers.openai]
api_key = "x"
"#,
        )
        .expect("parse config");

        assert_eq!(cfg.max_concurrent_sessions, 8);

        assert!(cfg.shell.enabled);
        assert_eq!(cfg.shell.timeout_secs, 30);
        assert_eq!(cfg.shell.max_output_bytes, 10_000);

        assert!(cfg.process.enabled);
        assert_eq!(cfg.process.max_concurrent, 8);
        assert_eq!(cfg.process.max_output_bytes, 524_288);
        assert_eq!(cfg.process.cleanup_retention_secs, 600);
        assert_eq!(cfg.process.kill_grace_secs, 5);
        assert_eq!(cfg.process.wait_default_secs, 20);
        assert_eq!(cfg.process.wait_max_secs, 25);
    }

    #[test]
    fn validate_rejects_zero_max_concurrent_sessions() {
        let cfg: AppConfig = toml::from_str(
            r#"
provider = "openai"
max_concurrent_sessions = 0
[providers.openai]
api_key = "x"
"#,
        )
        .expect("parse config");

        let err = cfg.validate().unwrap_err().to_string();
        assert!(err.contains("max_concurrent_sessions"));
    }

    #[test]
    fn shell_accepts_timeout_seconds_alias() {
        let cfg: AppConfig = toml::from_str(
            r#"
provider = "openai"
[providers.openai]
api_key = "x"

[shell]
timeout_seconds = 42
"#,
        )
        .expect("parse config");

        assert_eq!(cfg.shell.timeout_secs, 42);
    }

    #[test]
    fn shell_resolved_timeout_is_capped() {
        let cfg = ShellConfig {
            enabled: true,
            timeout_secs: 9_999,
            max_output_bytes: 10_000,
        };
        assert_eq!(cfg.resolved_timeout_secs(), 3_600);
    }

    #[test]
    fn process_resolved_max_concurrent_is_capped() {
        let cfg = ProcessConfig {
            enabled: true,
            max_concurrent: 9_999,
            max_output_bytes: 524_288,
            cleanup_retention_secs: 600,
            kill_grace_secs: 5,
            wait_default_secs: 20,
            wait_max_secs: 25,
        };
        assert_eq!(cfg.resolved_max_concurrent(), 128);
    }

    #[test]
    fn validate_rejects_process_wait_default_above_wait_max() {
        let cfg: AppConfig = toml::from_str(
            r#"
provider = "openai"
[providers.openai]
api_key = "x"

[process]
wait_default_secs = 30
wait_max_secs = 20
"#,
        )
        .expect("parse config");

        let err = cfg.validate().unwrap_err().to_string();
        assert!(err.contains("wait_default_secs"));
        assert!(err.contains("less than or equal to wait_max_secs"));
    }
}

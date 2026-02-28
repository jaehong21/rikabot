use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, bail, Context, Result};
use base64::Engine;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::time::{timeout, Duration};

use crate::config::{McpAuthMode, McpServerConfig};

const OAUTH_CALLBACK_PATH: &str = "/oauth/mcp/callback";
const OAUTH_CALLBACK_TIMEOUT_SECS: u64 = 300;

#[derive(Debug, Clone)]
pub struct McpOAuthSession {
    server_name: String,
    mcp_url: String,
    auth_server_override: Option<String>,
    configured_client_id: Option<String>,
    configured_client_secret: Option<String>,
    configured_scopes: Vec<String>,
    state_path: PathBuf,
    state: OAuthPersistedState,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct OAuthPersistedState {
    client_id: Option<String>,
    client_secret: Option<String>,
    authorization_server: Option<String>,
    access_token: Option<String>,
    refresh_token: Option<String>,
    token_type: Option<String>,
    scope: Option<String>,
    expires_at_epoch_secs: Option<i64>,
}

#[derive(Debug, Clone, Deserialize)]
struct OAuthProtectedResourceMetadata {
    #[serde(default)]
    authorization_servers: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct OAuthAuthorizationServerMetadata {
    pub issuer: String,
    pub authorization_endpoint: String,
    pub token_endpoint: String,
    pub registration_endpoint: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct OAuthTokenResponse {
    access_token: String,
    token_type: Option<String>,
    refresh_token: Option<String>,
    expires_in: Option<u64>,
    scope: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct OAuthClientRegistrationResponse {
    client_id: String,
    client_secret: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct BearerChallenge {
    params: HashMap<String, String>,
}

impl BearerChallenge {
    pub fn get(&self, key: &str) -> Option<&str> {
        self.params
            .get(&key.to_ascii_lowercase())
            .map(std::string::String::as_str)
    }
}

impl McpOAuthSession {
    pub fn new(config: &McpServerConfig, workspace_dir: &Path) -> Result<Option<Self>> {
        if config.auth_mode != McpAuthMode::Oauth {
            return Ok(None);
        }

        let mcp_url = config
            .url
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .ok_or_else(|| anyhow!("oauth mode requires MCP server url"))?
            .to_string();

        let mut token_dir = workspace_dir.to_path_buf();
        token_dir.push(".mcp");
        token_dir.push("oauth");
        std::fs::create_dir_all(&token_dir).with_context(|| {
            format!(
                "failed to create MCP OAuth token directory '{}'",
                token_dir.display()
            )
        })?;

        let file_name = format!("{}.json", sanitize_file_component(&config.name));
        let state_path = token_dir.join(file_name);

        let mut state = match load_state(&state_path) {
            Ok(state) => state,
            Err(err) => {
                tracing::warn!(
                    "Failed to load MCP OAuth state for server '{}': {}. Starting with empty OAuth state.",
                    config.name,
                    err
                );
                OAuthPersistedState::default()
            }
        };
        let configured_client_secret = config.resolved_oauth_client_secret()?;
        let configured_client_id = config.oauth_client_id.clone();
        let auth_server_override = config.oauth_authorization_server.clone();
        let configured_scopes = config
            .oauth_scopes
            .iter()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>();

        if let Some(client_id) = configured_client_id.clone() {
            state.client_id = Some(client_id);
        }
        if let Some(secret) = configured_client_secret.clone() {
            state.client_secret = Some(secret);
        }
        if let Some(auth_server) = auth_server_override.clone() {
            state.authorization_server = Some(auth_server);
        }

        Ok(Some(Self {
            server_name: config.name.clone(),
            mcp_url,
            auth_server_override,
            configured_client_id,
            configured_client_secret,
            configured_scopes,
            state_path,
            state,
        }))
    }

    pub fn authorization_header_value(&self) -> Option<String> {
        self.state
            .access_token
            .as_ref()
            .filter(|token| !token.trim().is_empty())
            .map(|token| format!("Bearer {}", token))
    }

    pub async fn maybe_refresh_token(&mut self, http_client: &reqwest::Client) -> Result<()> {
        if self.state.access_token.is_none() {
            return Ok(());
        }
        let Some(expires_at) = self.state.expires_at_epoch_secs else {
            return Ok(());
        };
        if now_epoch_secs() + 30 < expires_at {
            return Ok(());
        }

        if self.state.refresh_token.is_none() {
            return Ok(());
        }

        let metadata = self
            .resolve_authorization_server_metadata(http_client, None)
            .await?;
        self.refresh_access_token(http_client, &metadata).await?;
        self.persist_state()?;
        Ok(())
    }

    pub async fn recover_from_401(
        &mut self,
        http_client: &reqwest::Client,
        www_authenticate: Option<&str>,
    ) -> Result<bool> {
        let challenge = www_authenticate
            .and_then(parse_bearer_www_authenticate)
            .unwrap_or_default();

        let metadata = self
            .resolve_authorization_server_metadata(http_client, Some(&challenge))
            .await?;

        if self.state.refresh_token.is_some() && self.state.client_id.is_some() {
            if let Err(err) = self.refresh_access_token(http_client, &metadata).await {
                tracing::warn!(
                    "MCP OAuth refresh failed for server '{}': {}",
                    self.server_name,
                    err
                );
            } else {
                self.persist_state()?;
                return Ok(true);
            }
        }

        self.authorize_interactive(http_client, &metadata, &challenge)
            .await?;
        self.persist_state()?;
        Ok(true)
    }

    async fn authorize_interactive(
        &mut self,
        http_client: &reqwest::Client,
        metadata: &OAuthAuthorizationServerMetadata,
        challenge: &BearerChallenge,
    ) -> Result<()> {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .context("failed to bind OAuth loopback callback listener")?;
        let callback_addr = listener
            .local_addr()
            .context("failed to read OAuth callback listener address")?;
        let redirect_uri = format!(
            "http://127.0.0.1:{}{}",
            callback_addr.port(),
            OAUTH_CALLBACK_PATH
        );

        let (client_id, client_secret) = self
            .resolve_or_register_client(http_client, metadata, &redirect_uri)
            .await?;

        let pkce_verifier = random_token(32);
        let code_challenge = pkce_s256_challenge(&pkce_verifier);
        let state = random_token(24);
        let scope = merge_scopes(&self.configured_scopes, challenge.get("scope"));

        let authorize_url = build_authorize_url(
            &metadata.authorization_endpoint,
            &client_id,
            &redirect_uri,
            &code_challenge,
            &state,
            &self.mcp_url,
            scope.as_deref(),
        )?;

        tracing::info!(
            "Starting MCP OAuth flow for server '{}' with provider '{}'",
            self.server_name,
            metadata.issuer
        );
        if let Err(err) = webbrowser::open(authorize_url.as_str()) {
            tracing::warn!(
                "Failed to auto-open browser for MCP OAuth server '{}': {}. Open this URL manually: {}",
                self.server_name,
                err,
                authorize_url
            );
        } else {
            tracing::info!(
                "Opened browser for MCP OAuth server '{}': {}",
                self.server_name,
                authorize_url
            );
        }

        let callback = wait_for_callback(listener, OAUTH_CALLBACK_PATH).await?;
        if callback.state != state {
            bail!("OAuth callback state mismatch");
        }

        let mut form = vec![
            ("grant_type", "authorization_code".to_string()),
            ("code", callback.code),
            ("redirect_uri", redirect_uri),
            ("client_id", client_id.clone()),
            ("code_verifier", pkce_verifier),
        ];
        if let Some(secret) = client_secret.clone() {
            form.push(("client_secret", secret));
        }

        let token_response: OAuthTokenResponse = post_form_json(
            http_client,
            &metadata.token_endpoint,
            &form,
            "OAuth authorization-code token exchange failed",
        )
        .await?;

        self.state.access_token = Some(token_response.access_token);
        if let Some(refresh_token) = token_response.refresh_token {
            self.state.refresh_token = Some(refresh_token);
        }
        self.state.token_type = token_response.token_type;
        self.state.scope = token_response.scope.or(scope);
        self.state.expires_at_epoch_secs = token_response
            .expires_in
            .map(|expires_in| now_epoch_secs() + expires_in as i64);
        self.state.client_id = Some(client_id);
        if let Some(secret) = client_secret {
            self.state.client_secret = Some(secret);
        }
        self.state.authorization_server = Some(metadata.issuer.clone());
        Ok(())
    }

    async fn refresh_access_token(
        &mut self,
        http_client: &reqwest::Client,
        metadata: &OAuthAuthorizationServerMetadata,
    ) -> Result<()> {
        let refresh_token = self
            .state
            .refresh_token
            .clone()
            .ok_or_else(|| anyhow!("refresh token is not available"))?;
        let client_id = self
            .effective_client_id()
            .ok_or_else(|| anyhow!("client_id is not available for refresh"))?;

        let mut form = vec![
            ("grant_type", "refresh_token".to_string()),
            ("refresh_token", refresh_token),
            ("client_id", client_id),
        ];
        if let Some(secret) = self.effective_client_secret() {
            form.push(("client_secret", secret));
        }

        let token_response: OAuthTokenResponse = post_form_json(
            http_client,
            &metadata.token_endpoint,
            &form,
            "OAuth refresh token exchange failed",
        )
        .await?;

        self.state.access_token = Some(token_response.access_token);
        if let Some(next_refresh_token) = token_response.refresh_token {
            self.state.refresh_token = Some(next_refresh_token);
        }
        self.state.token_type = token_response.token_type;
        if token_response.scope.is_some() {
            self.state.scope = token_response.scope;
        }
        self.state.expires_at_epoch_secs = token_response
            .expires_in
            .map(|expires_in| now_epoch_secs() + expires_in as i64);
        Ok(())
    }

    async fn resolve_or_register_client(
        &mut self,
        http_client: &reqwest::Client,
        metadata: &OAuthAuthorizationServerMetadata,
        redirect_uri: &str,
    ) -> Result<(String, Option<String>)> {
        if let Some(client_id) = self.effective_client_id() {
            return Ok((client_id, self.effective_client_secret()));
        }

        let Some(registration_endpoint) = metadata.registration_endpoint.as_deref() else {
            bail!(
                "OAuth client_id is missing for server '{}' and registration endpoint is unavailable",
                self.server_name
            );
        };

        let body = serde_json::json!({
            "client_name": format!("rikabot-mcp-{}", self.server_name),
            "redirect_uris": [redirect_uri],
            "grant_types": ["authorization_code", "refresh_token"],
            "response_types": ["code"],
            "token_endpoint_auth_method": "none",
        });

        let resp = http_client
            .post(registration_endpoint)
            .header("Accept", "application/json")
            .json(&body)
            .send()
            .await
            .context("OAuth dynamic client registration request failed")?;
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            if text.is_empty() {
                bail!("OAuth dynamic client registration failed: HTTP {}", status);
            }
            bail!(
                "OAuth dynamic client registration failed: HTTP {}: {}",
                status,
                text
            );
        }

        let payload: OAuthClientRegistrationResponse = resp
            .json()
            .await
            .context("failed to parse dynamic client registration response")?;
        self.state.client_id = Some(payload.client_id.clone());
        if let Some(secret) = payload.client_secret.clone() {
            self.state.client_secret = Some(secret.clone());
        }

        Ok((payload.client_id, payload.client_secret))
    }

    async fn resolve_authorization_server_metadata(
        &self,
        http_client: &reqwest::Client,
        challenge: Option<&BearerChallenge>,
    ) -> Result<OAuthAuthorizationServerMetadata> {
        if let Some(resource_metadata_url) = challenge.and_then(|c| c.get("resource_metadata")) {
            if let Some(metadata) =
                fetch_auth_metadata_from_protected_resource(http_client, resource_metadata_url)
                    .await?
            {
                return Ok(metadata);
            }
        }

        if let Some(metadata) = fetch_auth_metadata_from_protected_resource(
            http_client,
            &format!(
                "{}/.well-known/oauth-protected-resource",
                self.mcp_origin()?
            ),
        )
        .await?
        {
            return Ok(metadata);
        }

        if let Some(override_url) = self.auth_server_override.as_deref() {
            if let Some(metadata) =
                try_fetch_auth_server_metadata(http_client, override_url).await?
            {
                return Ok(metadata);
            }
        }

        if let Some(saved_auth_server) = self.state.authorization_server.as_deref() {
            if let Some(metadata) =
                try_fetch_auth_server_metadata(http_client, saved_auth_server).await?
            {
                return Ok(metadata);
            }
        }

        let fallback_url = format!(
            "{}/.well-known/oauth-authorization-server",
            self.mcp_origin()?
        );
        if let Some(metadata) = try_fetch_auth_server_metadata(http_client, &fallback_url).await? {
            return Ok(metadata);
        }

        bail!(
            "failed to discover OAuth authorization metadata for MCP server '{}'",
            self.server_name
        );
    }

    fn persist_state(&self) -> Result<()> {
        let data = serde_json::to_vec_pretty(&self.state)
            .context("failed to serialize MCP OAuth persisted state")?;
        let tmp_path = self.state_path.with_extension("json.tmp");
        std::fs::write(&tmp_path, data).with_context(|| {
            format!(
                "failed to write temporary MCP OAuth state '{}'",
                tmp_path.display()
            )
        })?;
        std::fs::rename(&tmp_path, &self.state_path).with_context(|| {
            format!(
                "failed to persist MCP OAuth state '{}'",
                self.state_path.display()
            )
        })?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(&self.state_path)
                .context("failed to read MCP OAuth state file metadata")?
                .permissions();
            perms.set_mode(0o600);
            std::fs::set_permissions(&self.state_path, perms)
                .context("failed to set MCP OAuth state file permissions")?;
        }
        Ok(())
    }

    fn effective_client_id(&self) -> Option<String> {
        self.configured_client_id
            .clone()
            .or_else(|| self.state.client_id.clone())
            .filter(|v| !v.trim().is_empty())
    }

    fn effective_client_secret(&self) -> Option<String> {
        self.configured_client_secret
            .clone()
            .or_else(|| self.state.client_secret.clone())
            .filter(|v| !v.trim().is_empty())
    }

    fn mcp_origin(&self) -> Result<String> {
        let parsed = reqwest::Url::parse(&self.mcp_url)
            .with_context(|| format!("invalid MCP url '{}'", self.mcp_url))?;
        let host = parsed
            .host_str()
            .ok_or_else(|| anyhow!("MCP url has no host: {}", self.mcp_url))?;
        let origin = match parsed.port() {
            Some(port) => format!("{}://{}:{}", parsed.scheme(), host, port),
            None => format!("{}://{}", parsed.scheme(), host),
        };
        Ok(origin)
    }
}

pub fn parse_bearer_www_authenticate(value: &str) -> Option<BearerChallenge> {
    let trimmed = value.trim();
    if trimmed.len() < 6 || !trimmed[..6].eq_ignore_ascii_case("bearer") {
        return None;
    }
    let params = trimmed[6..].trim();
    if params.is_empty() {
        return Some(BearerChallenge::default());
    }

    let mut parsed = HashMap::new();
    for part in split_comma_preserving_quotes(params) {
        let segment = part.trim();
        if segment.is_empty() {
            continue;
        }
        let Some((raw_key, raw_value)) = segment.split_once('=') else {
            continue;
        };
        let key = raw_key.trim().to_ascii_lowercase();
        if key.is_empty() {
            continue;
        }
        let mut value = raw_value.trim().to_string();
        if value.starts_with('"') && value.ends_with('"') && value.len() >= 2 {
            value = value[1..value.len() - 1].to_string();
        }
        value = value.replace("\\\"", "\"");
        value = value.replace("\\\\", "\\");
        parsed.insert(key, value);
    }

    Some(BearerChallenge { params: parsed })
}

fn split_comma_preserving_quotes(input: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut start = 0usize;
    let mut in_quotes = false;
    let mut escape = false;

    for (idx, ch) in input.char_indices() {
        if escape {
            escape = false;
            continue;
        }
        match ch {
            '\\' => {
                if in_quotes {
                    escape = true;
                }
            }
            '"' => {
                in_quotes = !in_quotes;
            }
            ',' if !in_quotes => {
                parts.push(input[start..idx].to_string());
                start = idx + 1;
            }
            _ => {}
        }
    }
    parts.push(input[start..].to_string());
    parts
}

fn merge_scopes(configured_scopes: &[String], challenge_scope: Option<&str>) -> Option<String> {
    let mut set = BTreeSet::new();
    for value in configured_scopes {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            set.insert(trimmed.to_string());
        }
    }
    if let Some(scope) = challenge_scope {
        for token in scope.split_whitespace() {
            let trimmed = token.trim();
            if !trimmed.is_empty() {
                set.insert(trimmed.to_string());
            }
        }
    }
    if set.is_empty() {
        None
    } else {
        Some(set.into_iter().collect::<Vec<_>>().join(" "))
    }
}

fn build_authorize_url(
    authorization_endpoint: &str,
    client_id: &str,
    redirect_uri: &str,
    code_challenge: &str,
    state: &str,
    resource: &str,
    scope: Option<&str>,
) -> Result<reqwest::Url> {
    let mut url = reqwest::Url::parse(authorization_endpoint).with_context(|| {
        format!(
            "invalid authorization endpoint '{}'",
            authorization_endpoint
        )
    })?;
    {
        let mut query = url.query_pairs_mut();
        query.append_pair("response_type", "code");
        query.append_pair("client_id", client_id);
        query.append_pair("redirect_uri", redirect_uri);
        query.append_pair("code_challenge", code_challenge);
        query.append_pair("code_challenge_method", "S256");
        query.append_pair("state", state);
        query.append_pair("resource", resource);
        if let Some(scope) = scope {
            query.append_pair("scope", scope);
        }
    }
    Ok(url)
}

async fn wait_for_callback(listener: TcpListener, expected_path: &str) -> Result<OAuthCallback> {
    let (mut stream, _) = timeout(
        Duration::from_secs(OAUTH_CALLBACK_TIMEOUT_SECS),
        listener.accept(),
    )
    .await
    .context("timed out waiting for OAuth browser callback")?
    .context("failed to accept OAuth browser callback connection")?;

    let mut buf = vec![0u8; 16 * 1024];
    let n = timeout(Duration::from_secs(10), stream.read(&mut buf))
        .await
        .context("timed out reading OAuth callback request")?
        .context("failed to read OAuth callback request")?;

    if n == 0 {
        bail!("received empty OAuth callback request");
    }

    let request_text = String::from_utf8_lossy(&buf[..n]);
    let first_line = request_text
        .lines()
        .next()
        .ok_or_else(|| anyhow!("invalid OAuth callback request line"))?;

    let mut line_parts = first_line.split_whitespace();
    let method = line_parts.next().unwrap_or("");
    let target = line_parts.next().unwrap_or("");

    let parsed_url = reqwest::Url::parse(&format!("http://localhost{}", target))
        .context("failed to parse OAuth callback URL")?;

    if method != "GET" || parsed_url.path() != expected_path {
        send_callback_response(
            &mut stream,
            400,
            "Invalid callback request. You can close this tab.",
        )
        .await?;
        bail!(
            "unexpected OAuth callback target '{} {}'",
            method,
            parsed_url.path()
        );
    }

    let mut code = None;
    let mut state = None;
    let mut oauth_error = None;
    let mut oauth_error_description = None;
    for (k, v) in parsed_url.query_pairs() {
        match k.as_ref() {
            "code" => code = Some(v.into_owned()),
            "state" => state = Some(v.into_owned()),
            "error" => oauth_error = Some(v.into_owned()),
            "error_description" => oauth_error_description = Some(v.into_owned()),
            _ => {}
        }
    }

    if let Some(err) = oauth_error {
        send_callback_response(
            &mut stream,
            400,
            "OAuth authorization failed. You can close this tab.",
        )
        .await?;
        let description = oauth_error_description.unwrap_or_default();
        bail!("OAuth callback error: {} {}", err, description);
    }

    let code = code.ok_or_else(|| anyhow!("OAuth callback is missing 'code' parameter"))?;
    let state = state.ok_or_else(|| anyhow!("OAuth callback is missing 'state' parameter"))?;
    send_callback_response(
        &mut stream,
        200,
        "OAuth login completed. You can close this tab.",
    )
    .await?;

    Ok(OAuthCallback { code, state })
}

async fn send_callback_response(
    stream: &mut tokio::net::TcpStream,
    code: u16,
    body: &str,
) -> Result<()> {
    let status = match code {
        200 => "200 OK",
        400 => "400 Bad Request",
        _ => "500 Internal Server Error",
    };
    let html = format!("<!doctype html><html><body><p>{}</p></body></html>", body);
    let response = format!(
        "HTTP/1.1 {}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        status,
        html.len(),
        html
    );
    stream
        .write_all(response.as_bytes())
        .await
        .context("failed to write OAuth callback response")?;
    stream
        .flush()
        .await
        .context("failed to flush OAuth callback response")?;
    Ok(())
}

async fn post_form_json<T: for<'de> Deserialize<'de>>(
    http_client: &reqwest::Client,
    url: &str,
    form: &[(&str, String)],
    error_context: &str,
) -> Result<T> {
    let response = http_client
        .post(url)
        .header("Accept", "application/json")
        .form(form)
        .send()
        .await
        .with_context(|| error_context.to_string())?;
    if !response.status().is_success() {
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        if text.is_empty() {
            bail!("{}: HTTP {}", error_context, status);
        }
        bail!("{}: HTTP {}: {}", error_context, status, text);
    }
    response
        .json::<T>()
        .await
        .with_context(|| format!("{}: failed to parse JSON response", error_context))
}

async fn fetch_auth_metadata_from_protected_resource(
    http_client: &reqwest::Client,
    metadata_url: &str,
) -> Result<Option<OAuthAuthorizationServerMetadata>> {
    let response = http_client
        .get(metadata_url)
        .header("Accept", "application/json")
        .send()
        .await
        .with_context(|| {
            format!(
                "failed to request OAuth protected resource metadata '{}'",
                metadata_url
            )
        })?;

    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }
    if !response.status().is_success() {
        return Ok(None);
    }

    let payload: OAuthProtectedResourceMetadata = response.json().await.with_context(|| {
        format!(
            "failed to parse OAuth protected resource metadata '{}'",
            metadata_url
        )
    })?;
    let Some(first_auth_server) = payload.authorization_servers.first() else {
        return Ok(None);
    };

    try_fetch_auth_server_metadata(http_client, first_auth_server).await
}

async fn try_fetch_auth_server_metadata(
    http_client: &reqwest::Client,
    issuer_or_metadata_url: &str,
) -> Result<Option<OAuthAuthorizationServerMetadata>> {
    let candidate = candidate_metadata_url(issuer_or_metadata_url)?;
    let response = http_client
        .get(&candidate)
        .header("Accept", "application/json")
        .send()
        .await
        .with_context(|| {
            format!(
                "failed to request OAuth authorization server metadata '{}'",
                candidate
            )
        })?;

    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }
    if !response.status().is_success() {
        return Ok(None);
    }

    let payload: OAuthAuthorizationServerMetadata = response.json().await.with_context(|| {
        format!(
            "failed to parse OAuth authorization server metadata '{}'",
            candidate
        )
    })?;
    Ok(Some(payload))
}

fn candidate_metadata_url(issuer_or_metadata_url: &str) -> Result<String> {
    let parsed = reqwest::Url::parse(issuer_or_metadata_url)
        .with_context(|| format!("invalid OAuth URL '{}'", issuer_or_metadata_url))?;
    if parsed
        .path()
        .contains("/.well-known/oauth-authorization-server")
    {
        return Ok(issuer_or_metadata_url.to_string());
    }
    Ok(format!(
        "{}/.well-known/oauth-authorization-server",
        issuer_or_metadata_url.trim_end_matches('/')
    ))
}

fn load_state(path: &Path) -> Result<OAuthPersistedState> {
    if !path.exists() {
        return Ok(OAuthPersistedState::default());
    }
    let bytes = std::fs::read(path)
        .with_context(|| format!("failed to read MCP OAuth state '{}'", path.display()))?;
    let parsed = serde_json::from_slice::<OAuthPersistedState>(&bytes)
        .with_context(|| format!("failed to parse MCP OAuth state '{}'", path.display()))?;
    Ok(parsed)
}

fn sanitize_file_component(input: &str) -> String {
    let mut out = String::with_capacity(input.len().max(1));
    for ch in input.chars() {
        if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
            out.push(ch);
        } else {
            out.push('_');
        }
    }
    if out.is_empty() {
        "_".to_string()
    } else {
        out
    }
}

fn now_epoch_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

fn random_token(bytes: usize) -> String {
    let mut data = vec![0u8; bytes];
    rand::thread_rng().fill_bytes(&mut data);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(data)
}

fn pkce_s256_challenge(verifier: &str) -> String {
    let digest = Sha256::digest(verifier.as_bytes());
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(digest)
}

struct OAuthCallback {
    code: String,
    state: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::routing::get;
    use axum::Router;
    use tokio::net::TcpListener;

    #[test]
    fn parse_bearer_challenge_extracts_basic_fields() {
        let parsed = parse_bearer_www_authenticate(
            r#"Bearer realm="OAuth", error="invalid_token", scope="a b c", resource_metadata="https://example.com/.well-known/oauth-protected-resource""#,
        )
        .expect("should parse");

        assert_eq!(parsed.get("realm"), Some("OAuth"));
        assert_eq!(parsed.get("error"), Some("invalid_token"));
        assert_eq!(parsed.get("scope"), Some("a b c"));
        assert_eq!(
            parsed.get("resource_metadata"),
            Some("https://example.com/.well-known/oauth-protected-resource")
        );
    }

    #[test]
    fn parse_bearer_challenge_ignores_non_bearer_scheme() {
        assert!(parse_bearer_www_authenticate("Basic abc123").is_none());
    }

    #[test]
    fn merge_scopes_deduplicates_and_sorts() {
        let merged = merge_scopes(
            &["write".to_string(), "read".to_string()],
            Some("read admin"),
        )
        .expect("scope should exist");
        assert_eq!(merged, "admin read write");
    }

    #[tokio::test]
    async fn fetches_auth_server_metadata_from_issuer_or_metadata_url() {
        let app = Router::new().route(
            "/.well-known/oauth-authorization-server",
            get(|| async {
                axum::Json(serde_json::json!({
                    "issuer": "http://127.0.0.1",
                    "authorization_endpoint": "http://127.0.0.1/authorize",
                    "token_endpoint": "http://127.0.0.1/token",
                }))
            }),
        );
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        let client = reqwest::Client::new();
        let metadata = try_fetch_auth_server_metadata(&client, &format!("http://{}", addr))
            .await
            .unwrap()
            .expect("metadata should exist");
        assert_eq!(metadata.issuer, "http://127.0.0.1");

        task.abort();
    }
}

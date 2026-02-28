# MCP OAuth Support for HTTP Servers (Linear First)

## Context

Current MCP HTTP auth in Rikabot (from commits `4b8e1e525660951455a90a9e7a194d3bf92e7fda` and `9223dd211261d7b45aa988176aba620c54e6a8e7`) supports static headers only:

- `[[mcp.servers]].headers.Authorization = "Bearer ${LINEAR_API_KEY}"`
- Streamable HTTP transport with:
  - `Accept: application/json, text/event-stream`
  - `MCP-Session-Id` reuse

This works for explicit API-key/Bearer-token setups, but not for interactive OAuth authorization flows.

PRD `0009-mcp-support.md` explicitly scoped out interactive OAuth. This document adds OAuth support as a new phase.

## Feasibility Summary

OAuth flow support is feasible in the current project.

Evidence checked on **2026-02-28**:

1. MCP authorization draft requires OAuth 2.1 style behavior for protected resources (401 challenge handling, resource/auth server metadata discovery, authorization code + PKCE).
2. Linear MCP endpoint behavior:
   - `POST https://mcp.linear.app/mcp` without token returns `401` with `WWW-Authenticate: Bearer ... invalid_token`.
   - `https://mcp.linear.app/.well-known/oauth-authorization-server` returns valid OAuth AS metadata (`authorization_endpoint`, `token_endpoint`, `registration_endpoint`, `grant_types_supported`, `token_endpoint_auth_methods_supported` incl. `none`).
   - `https://mcp.linear.app/.well-known/oauth-protected-resource` currently returns `404`.

Conclusion:

- Yes, OAuth can work for Linear in Rikabot.
- We must implement metadata discovery fallback for servers that omit protected-resource metadata but expose authorization-server metadata.

## Goals

1. Support MCP OAuth authorization flow for HTTP MCP servers, starting with Linear compatibility.
2. Allow users to use MCP servers without manually placing long-lived tokens in `config.toml`.
3. Persist OAuth tokens and automatically refresh access tokens.
4. Keep backward compatibility with existing static `headers` auth mode.
5. Keep MCP startup resilient: OAuth-required servers should not break whole app startup.

## Non-Goals

- OAuth for `stdio` MCP servers.
- Replacing existing static header auth path.
- Multi-user identity/session separation (single local user model remains).
- Full OAuth provider management UI beyond MCP use cases.

## Functional Requirements

### 1) Auth Mode Model (Config)

Extend MCP server config to support explicit auth mode:

- `auth_mode = "headers" | "oauth"` (default: `headers`)
- Existing `headers` remains unchanged.
- `oauth` settings (optional when `auth_mode="oauth"`):
  - `client_id` (optional; required if dynamic registration disabled)
  - `client_secret_env` (optional; for confidential clients only)
  - `scopes` (optional list)
  - `authorization_server` (optional override URL)
  - `redirect_host` (default `127.0.0.1`)
  - `redirect_port` (default auto-allocated loopback port)
  - `redirect_path` (default `/oauth/mcp/callback`)
  - `dynamic_client_registration` (default `auto`)

Validation:

- reject invalid `auth_mode`
- reject `headers` + `oauth` conflicts that cannot be resolved
- enforce `https` for remote auth endpoints
- redact secrets in all logs/errors

### 2) OAuth Discovery and Bootstrap

For MCP HTTP servers with `auth_mode="oauth"`:

1. Attempt MCP request with stored access token (if any).
2. On `401` + Bearer challenge:
   - parse `WWW-Authenticate` parameters
   - discover metadata in this order:
     1. `resource_metadata` from challenge (if present)
     2. inferred protected-resource metadata URL from MCP server origin
     3. fallback to `/.well-known/oauth-authorization-server` on MCP server origin (required for Linear compatibility)
     4. explicit `oauth.authorization_server` override from config
3. Resolve authorization server metadata (`authorization_endpoint`, `token_endpoint`, optional `registration_endpoint`).
4. If `client_id` absent and registration endpoint exists, perform RFC7591 dynamic client registration.
5. Start authorization code flow with PKCE (`S256`), including `resource` parameter set to MCP resource URL.

### 3) User Authorization UX

Add a first-class OAuth login path with loopback callback:

- Start endpoint/action returns browser URL for authorization.
- Open browser automatically when possible; otherwise print URL for manual open.
- Local callback receiver captures `code` and validates `state`.
- Exchange code for access/refresh token.
- Show clear completion/failure status to user.

Initial scope:

- single-user local Rikabot process
- one credential set per MCP server name

### 4) Token Storage and Refresh

Persist per-server OAuth state:

- `access_token`
- `refresh_token` (if issued)
- `expires_at`
- `token_type`
- `scope`
- registration metadata (`client_id`, optional `client_secret`) when DCR is used

Storage requirements:

- dedicated local auth file path outside source control
- file permissions restrictive (owner-only on supported OS)
- never write secrets to logs

Refresh behavior:

1. refresh proactively when token is near expiry
2. refresh reactively on `401 invalid_token`
3. retry original MCP request once after successful refresh
4. if refresh fails, emit re-auth-required state

### 5) MCP Runtime Integration

Current `main.rs` connects MCP servers at startup. OAuth introduces interactive/auth-required states, so runtime must support:

- `connected`
- `auth_required`
- `connecting`
- `failed`

Behavior:

- startup continues even when OAuth is pending
- non-OAuth servers connect normally
- OAuth server tools are registered only after auth success and `tools/list` succeeds
- reconnect flow can be triggered without restarting app

### 6) Step-Up and Scope Handling

If MCP server returns `insufficient_scope` or scope hints in challenge:

- mark server as step-up required
- re-run OAuth with additional requested scopes
- after success, retry failed MCP request

## Architecture Changes

### New modules

- `src/tools/mcp_oauth.rs`
  - challenge parsing
  - metadata discovery
  - authorize URL generation (PKCE/state)
  - token exchange/refresh
- `src/tools/mcp_token_store.rs`
  - secure local persistence for OAuth credentials/client registration
- `src/tools/mcp_auth_state.rs`
  - per-server connection/auth lifecycle state

### Existing module updates

- `src/config.rs`
  - add MCP auth mode + OAuth config structs/validation
- `src/tools/mcp_transport.rs`
  - inject bearer access token dynamically
  - detect `401` auth failures and surface structured auth errors
- `src/tools/mcp_client.rs`
  - support deferred connect/auth-required states and reconnect
- `src/main.rs`
  - initialize MCP registry in stateful mode (not startup-fatal if OAuth pending)
- `src/gateway/mod.rs` and `src/gateway/ws.rs`
  - expose OAuth start/callback/status actions/events to web client
- `web/src/App.svelte`
  - render MCP auth-required status and “Connect” action for OAuth servers

## API / Event Changes

### Server -> Client events

- `mcp_status`
  - `{ server, status, reason? }`
- `mcp_oauth_required`
  - `{ server, authorize_url? }`
- `mcp_oauth_completed`
  - `{ server }`
- `mcp_oauth_failed`
  - `{ server, message }`

### Client -> Server messages

- `mcp_oauth_start`
  - `{ server }`
- `mcp_oauth_retry`
  - `{ server }`

### HTTP routes (gateway)

- `GET /oauth/mcp/callback` (loopback code receiver)
- optional `GET /api/mcp/:server/oauth/start` (returns authorization URL)

## Security Requirements

1. Use Authorization Code + PKCE (`S256`) for public clients.
2. Validate `state` on callback; reject mismatches.
3. Use loopback redirect URIs for local auth flow.
4. Do not log tokens, auth codes, client secrets, or full callback query.
5. Restrict token-store file permissions.
6. Keep TLS-only auth/token endpoints unless explicitly configured otherwise for local testing.

## Testing Requirements

Add tests for non-trivial OAuth behavior:

1. Parse `WWW-Authenticate` Bearer challenge correctly.
2. Discovery fallback path works when protected-resource metadata is missing (`404`) but authorization-server metadata exists on origin (Linear case).
3. PKCE generation + callback `state` validation.
4. Dynamic client registration parsing and persistence.
5. Access-token refresh and single retry on 401.
6. Step-up scope re-auth behavior.
7. Startup resilience: OAuth-required server does not block local tools or other MCP servers.
8. Web event/UI state transitions for auth-required -> connected.

## Implementation Plan

### Phase 1: Core OAuth engine

1. Add config/schema for MCP OAuth mode.
2. Implement challenge parsing + metadata discovery + fallback.
3. Implement PKCE, auth URL, token exchange, refresh.
4. Implement secure local token store.

### Phase 2: MCP integration

1. Wire bearer token injection into HTTP transport.
2. Add structured auth error handling and retry/refresh.
3. Refactor registry to support auth-required state and deferred connect.

### Phase 3: User flow (gateway + web)

1. Add OAuth start/callback/status routes/events.
2. Add web auth-required banner and connect action.
3. Add reconnect after successful auth without full process restart.

### Phase 4: Hardening

1. Add end-to-end tests with mock OAuth + MCP server.
2. Add observability (auth attempts, refresh count, auth failures).
3. Document operator setup for Linear OAuth mode.

## Acceptance Criteria

1. A Linear MCP server configured with `auth_mode="oauth"` can connect and list tools without static API key headers in `config.toml`.
2. First-time authorization completes via browser + callback and persists credentials locally.
3. Expired access tokens refresh automatically when possible.
4. If OAuth is missing/invalid, app starts and reports `auth_required` instead of failing startup.
5. Existing header-based MCP servers continue working unchanged.

## Decisions

1. MVP auto-opens the browser as soon as OAuth authorization is required.
2. OAuth tokens are stored under the workspace directory (`{workspace_dir}/.mcp/oauth`).

## Open Questions / Assumptions

1. Linear currently exposes OAuth AS metadata but not protected-resource metadata; implementation assumes origin fallback is acceptable for interoperability.
2. Loopback callback uses `http://127.0.0.1:{random_port}/oauth/mcp/callback`; this assumes local browser + app run on same machine.

## References

- MCP Authorization (draft): https://modelcontextprotocol.io/specification/draft/basic/authorization
- Linear MCP docs: https://linear.app/docs/mcp
- OAuth 2.0 Protected Resource Metadata (RFC9728): https://www.rfc-editor.org/rfc/rfc9728
- OAuth 2.0 Authorization Server Metadata (RFC8414): https://www.rfc-editor.org/rfc/rfc8414
- OAuth 2.0 Dynamic Client Registration (RFC7591): https://www.rfc-editor.org/rfc/rfc7591
- PKCE (RFC7636): https://www.rfc-editor.org/rfc/rfc7636

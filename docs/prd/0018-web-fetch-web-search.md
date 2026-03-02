# 018 — Web Fetch + Web Search Tools (OpenRouter)

## Context

Rikabot currently has filesystem and shell tools, but no first-class web tools.

User requirement for this phase:

1. Add `web_fetch` and `web_search` tools.
2. For `web_search`, use a provider/model that is **separate** from the agent's current global `provider` + `model`.
3. Initially support only:
   - `openrouter` (with OpenRouter web-search plugin)
4. Web domain access must be controlled via existing `permissions.tools.allow` / `permissions.tools.deny` rules (no new permission section).
5. MVP policy boundary:
   - `web_fetch`: domain-level allow/deny supported.
   - `web_search`: tool-level allow/deny only (domain-level policy deferred).

Reference patterns:

- nanobot: `/Users/jetty/Desktop/open-source/nanobot/nanobot/agent/tools/web.py`
- zeroclaw:
  - `/Users/jetty/Desktop/open-source/zeroclaw/src/tools/web_fetch.rs`
  - `/Users/jetty/Desktop/open-source/zeroclaw/src/tools/web_search_tool.rs`
  - `/Users/jetty/Desktop/open-source/zeroclaw/src/config/schema.rs`

Primary provider docs:

- OpenRouter web plugin: <https://openrouter.ai/docs/guides/features/plugins/web-search>
- OpenRouter plugin usage (`plugins: [{"id":"web"}]`) and plugin params (`max_results`, `search_prompt`, etc.)

## Goals

1. Add `web_fetch` for URL -> readable text extraction.
2. Add `web_search` for query -> search results.
3. Ensure `web_search` uses **dedicated provider/model config** and never silently reuses global agent model/provider.
4. Keep tool integration compatible with existing agent/provider loop (no protocol redesign).

## Non-Goals

1. Browser automation (`browser_open`, click/type/scroll) in this PR.
2. Adding more search providers (Brave/Tavily/Firecrawl/etc.) in this PR.
3. Reworking global provider factory or message schema.
4. Streaming search results.

## Scope

### In scope

1. New tool: `web_fetch`.
2. New tool: `web_search`.
3. New config sections for web tools in `config.toml` + `src/config.rs`.
4. Web tool registration in default tool registry.
5. Unit tests for config parsing/validation and core tool behavior.

### Out of scope

1. UI changes (tool UIs, dashboards).
2. MCP-based search.
3. Background indexing/caching.

## Functional Requirements

### 1) `web_fetch`

Tool contract (MVP):

- Name: `web_fetch`
- Params:
  - `url` (required, string)
  - `max_chars` (optional, integer, clamp range)
  - `extract_mode` (optional, enum: `text` | `markdown`; default `text`)

Behavior:

1. Accept only `http`/`https` URLs.
2. Enforce domain allow/block policy from `permissions` config before network request.
3. Fetch with configurable timeout and user-agent.
4. Handle redirects safely (bounded redirect count).
5. Convert HTML to readable text (or markdown mode if requested).
6. Truncate output to configured max size.
7. Return clear tool errors for invalid URLs, blocked domains, network failures, unsupported content types.

### 2) `web_search`

Tool contract (MVP):

- Name: `web_search`
- Params:
  - `query` (required, string)
  - `count` (optional, integer; clamp 1..10)

Behavior:

1. Resolve provider from `[web_search].provider` (currently only `openrouter`).
2. Resolve provider-specific model from dedicated web-search config.
3. Never fall back to global `[provider]/model` for search calls.
4. Return normalized result text:
   - header with provider + model
   - numbered list of title/url/snippet
   - no-result message when empty

Provider behavior (MVP):

1. `openrouter`
   - Call OpenRouter chat completions endpoint with selected model.
   - Use fixed API base URL `https://openrouter.ai/api/v1`.
   - Enable web search plugin via `plugins: [{"id":"web"}]`.
   - Include optional plugin params from config (for example `max_results`, `search_prompt`).
   - Parse returned answer + citations/links into normalized output.

### 3) Permissions integration (required)

Use the existing permissions engine and rule syntax only.

1. No new `[permissions.web]` config section.
2. `permissions.tools.allow` / `permissions.tools.deny` control both tools.
3. Supported rules in MVP:
   - `web_fetch(domain:docs.openclaw.ai)`
   - `web_search(*)`
4. Matching behavior:
   - `deny` rules still take precedence over `allow`.
   - when `permissions.enabled = true` and `allow` is empty, existing default deny behavior remains.
5. Domain selector matching (MVP):
   - `web_fetch`: derived from `url` host (lowercased, port stripped) before permission evaluation.
   - `web_search`: not supported in MVP.

## Config Model

Add top-level web tool config in `src/config.rs`.

```toml
[web_fetch]
enabled = false
timeout_secs = 20
max_response_size = 50000
user_agent = "rikabot/0.1"

[web_search]
enabled = false
provider = "openrouter"
max_results = 5
timeout_secs = 15
user_agent = "rikabot/0.1"

[web_search.providers.openrouter]
api_key = ""
env_key = "OPENROUTER_API_KEY"
model = "openai/gpt-4o-mini"
# optional plugin tuning
plugin_max_results = 5
plugin_search_prompt = ""

[permissions]
enabled = true

[permissions.tools]
allow = [
  "web_fetch(domain:docs.openclaw.ai)",
  "web_search(*)",
]
deny = [
  "web_fetch(domain:*.internal.local)",
  "web_search(*)", # example: full web_search shutdown
]
```

Validation rules:

1. If `web_search.enabled = true`, `provider` must be `openrouter`.
2. Selected provider model must be non-empty.
3. API key resolution uses `api_key` or `env_key` env var lookup.
4. `max_results` clamp to 1..10.
5. Timeout values must be > 0.
6. Permissions selector rules are validated by the existing rule compiler (`tool(selector:pattern)` grammar).

## Implementation Plan

### Phase 1: Config + `web_fetch`

1. Extend `AppConfig` with `web_fetch` + `web_search` sections only (no new permissions section).
2. Add defaults and validation helpers.
3. Implement `src/tools/web_fetch.rs` with URL-host extraction used for permission selector matching.
4. Register `web_fetch` in `src/tools/mod.rs` default registry.

### Phase 2: `web_search` with dedicated provider/model

1. Implement `src/tools/web_search.rs` using OpenRouter provider only.
2. Use provider-specific model from web-search config only.
3. Add provider HTTP client and response parser.
4. Register `web_search` in tool registry.

### Phase 3: `web_search` domain policy follow-up (non-MVP)

1. Add optional `domain` arg and provider-side scoping hooks.
2. Post-validate returned citations/URLs against policy.
3. Decide behavior for mixed-domain results (filter vs hard-fail).

### Phase 4: Tests + docs

1. Unit tests for:
   - config parse/default/validation
   - web_search provider validation + model isolation
   - response parsing for OpenRouter
   - input validation and error mapping
2. Update `config.toml` template comments with new sections.

## File-by-File Changes

1. `src/config.rs`
   - add config structs + defaults + validation for `web_fetch` and `web_search`
2. `src/tools/web_fetch.rs` (new)
3. `src/tools/web_search.rs` (new)
4. `src/tools/mod.rs`
   - register both tools
   - (if needed) accept config in `default_registry(...)`
5. `src/main.rs`
   - pass config into tool registry constructor if registry signature changes
6. `src/permissions/mod.rs` and/or `src/tools/mod.rs` (if needed)
   - evaluate `web_fetch` domain selectors against URL host
7. `config.toml`
   - add commented examples for new sections

## Acceptance Criteria

1. With `web_fetch.enabled = true`, model can call `web_fetch` and retrieve readable content for valid URLs.
2. Invalid/blocked URLs return explicit tool errors.
3. Existing permission rules can gate by domain for `web_fetch` using selectors such as `web_fetch(domain:docs.openclaw.ai)`.
4. Existing permission rules can gate `web_search` at tool level via `web_search(*)`.
5. With `provider = "openrouter"`, search works using OpenRouter model and web plugin.
6. Changing global `provider`/`model` does not change `web_search` provider/model behavior.
7. New config sections parse successfully with defaults.
8. Existing agent/provider/tool flow remains backward compatible when web tools are disabled.

## Risks and Mitigations

1. Risk: provider response shapes evolve (especially citations format).
   - Mitigation: tolerant parser + focused unit tests with fixture variants.
2. Risk: OpenRouter plugin args differ by model/provider route.
   - Mitigation: keep plugin config minimal in MVP and expose raw error messages.
3. Risk: SSRF/security concerns in fetch.
   - Mitigation: URL validation + domain-based permission rules in existing `allow/deny` + timeout/size caps.
4. Risk: false sense of domain control for search if only provider hints are used.
   - Mitigation: keep `web_search` domain policy out of MVP until citation post-validation is implemented.

## Open Questions

1. For OpenRouter `web_search`, do we want strict citation extraction (URL list only) or pass through full provider text when citations are missing?
2. For domain-aware `web_search` follow-up, should mixed-domain results be filtered or hard-failed?
3. Should we keep explicit `provider` in config while only `openrouter` is supported, or collapse to implicit provider for now?

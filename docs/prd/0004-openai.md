# OpenAI Provider (`/chat/completions`)

## Context

Rikabot currently supports:

- `openrouter` via `src/providers/openrouter.rs`
- `openai_codex` placeholder (not implemented) via `src/providers/openai_codex.rs`

There is no standard `openai` provider yet. We need a first-class provider that calls OpenAI Chat Completions (`/chat/completions`) and supports overriding `base_url` for compatible gateways/proxies.

Reference implementation:

- `../../open-source/zeroclaw/src/providers/openai.rs`

## Goals

1. Add a new `openai` provider implementation under `src/providers/`.
2. Use OpenAI Chat Completions API (`POST {base_url}/chat/completions`).
3. Support native tool calling with current `ToolSpec`/`ToolCall` flow.
4. Support configurable `base_url` override (not hardcoded only).
5. Wire provider selection through config and `create_provider`.

## Non-Goals

- Implementing `responses` API fallback.
- Implementing streaming/SSE in this PRD.
- Reworking the agent tool-call encoding model.
- Completing `openai_codex`.

## Requirements

### Provider behavior

- New file: `src/providers/openai.rs`.
- Provider struct stores:
  - API key
  - HTTP client
  - `base_url`
- Default base URL: `https://api.openai.com/v1`.
- `base_url` must be a base URL only (must **not** include `/chat/completions`).
- Request endpoint is always composed as `{base_url}/chat/completions` (trim trailing slash on base URL).
- Auth header: `Authorization: Bearer <api_key>`.
- Request body:
  - `model`
  - `messages`
  - `temperature`
  - optional `tools`
  - optional `tool_choice = "auto"` when tools are present

### Tool and message compatibility

- Maintain parity with current OpenRouter message/tool conversion strategy so agent behavior remains unchanged:
  - parse assistant tool-calls embedded in content JSON
  - parse tool-result messages from content JSON
  - map API `tool_calls` back into internal `ToolCall`

### Configuration

Add `[providers.openai]` config:

- `api_key` (optional)
- `env_key` (optional)
- `base_url` (optional)

Resolution order:

1. `api_key` then `env_key` for credential
2. `base_url` for endpoint override
3. fallback base URL to `https://api.openai.com/v1`

### Provider factory wiring

- Register module in `src/providers/mod.rs`.
- Add `"openai"` branch in `create_provider`.
- Return clear config errors when `provider = "openai"` but `[providers.openai]` is missing or invalid.

## Plan

### Phase 1: config + provider skeleton

1. Add `OpenAiConfig` and `providers.openai` to `src/config.rs`.
2. Add resolver helpers:
   - `resolve_api_key()`
   - `resolve_base_url()`
3. Add provider module and constructor with `base_url` override.

### Phase 2: chat completion implementation

1. Implement OpenAI request/response structs.
2. Reuse conversion logic pattern from `openrouter.rs` for tools/messages.
3. Implement `Provider` trait:
   - `supports_native_tools() -> true`
   - `chat(...) -> ChatResponse`

### Phase 3: wiring + tests

1. Add factory match arm for `"openai"`.
2. Add unit tests for:
   - config resolution (`api_key`, `env_key`, `base_url`)
   - URL composition with base URL override and trailing slash trimming
   - validation that `base_url` does not include `/chat/completions`
   - tool conversion and response parsing parity

## File-by-File Changes

- `src/providers/openai.rs` (new)
- `src/providers/mod.rs`
- `src/config.rs`
- tests in:
  - `src/providers/openai.rs`
  - `src/config.rs` (or existing test modules)

## Acceptance Criteria

1. Setting `provider = "openai"` works with valid `[providers.openai]`.
2. Requests are sent to `{resolved_base_url}/chat/completions`.
3. If `base_url` is unset, default endpoint is `https://api.openai.com/v1/chat/completions`.
4. Tool calls round-trip between agent/internal format and OpenAI native format.
5. `cargo test` passes for new and existing tests.

## Risks and Mitigations

- Risk: behavior drift vs `openrouter` message/tool JSON handling.
  - Mitigation: copy proven conversion logic pattern and add parity tests.
- Risk: malformed override URL.
  - Mitigation: validate URL shape in config resolver and fail early with actionable error.

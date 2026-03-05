# Rika

**Rika** — **R**ecursive **I**ntelligent **K**nowledge-based **A**gent

Rika is a lightweight, local-first AI assistant platform.

Its architecture is inspired by [nanobot](https://github.com/HKUDS/nanobot), and its Rust implementation style is inspired by [zeroclaw](https://github.com/zeroclaw-labs/zeroclaw).

## Single-Owner Notes

Rika is currently an implementation just for `@me` (`jaehong21`). It does not implement strict single-tenant enforcement by default; verify it for your use case before use. Its implementation can change frequently and may include breaking changes.

## What Rika does

- Maintains chat sessions and persistent memory in a workspace.
- Routes messages through a tool-enabled agent loop (shell + filesystem + MCP integrations).
- Includes permission controls and explicit approval flows for sensitive operations.
- Supports MCP server connections (HTTP and stdio transport), also with OAuth authentication.

## Frontend E2E Testing

Playwright E2E tests run against:

- real frontend dev server,
- real Rust backend WebSocket server,
- local mock OpenAI-compatible endpoint for deterministic agent responses.

Commands:

```bash
mise run test:e2e
```

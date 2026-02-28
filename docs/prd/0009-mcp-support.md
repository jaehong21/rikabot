# MCP Support (`stdio` + `http`)

## Context

Rikabot currently exposes only local tools (`shell`, `filesystem_*`) via `ToolRegistry`.

We need first-class MCP (Model Context Protocol) support so external tool servers can be configured and used by the existing agent loop.

Requested scope: both `stdio` and `http` transports.

Primary near-term hosted MCP targets:

- Notion MCP: `https://mcp.notion.com/mcp`
- Linear MCP: `https://mcp.linear.app/mcp`

Reference implementations reviewed:

- `nanobot`
  - `nanobot/agent/tools/mcp.py` (`stdio` + streamable HTTP MCP client, tool wrapping)
- `zeroclaw`
  - `src/tools/mcp_protocol.rs` (JSON-RPC protocol types)
  - `src/tools/mcp_transport.rs` (`stdio`/`http` transport abstraction)
  - `src/tools/mcp_client.rs` (initialize + tools/list + tools/call flow)
  - `src/tools/mcp_tool.rs` (tool wrapper into existing registry)

## Goals

1. Add MCP client support in Rikabot for `stdio` and `http` transports.
2. Discover MCP tools dynamically from configured servers.
3. Expose discovered MCP tools through existing `ToolRegistry` and agent loop without changing provider contracts.
4. Keep startup resilient: one bad MCP server must not prevent app startup.
5. Provide clear config and validation for both transport types.
6. Support secure token headers for hosted HTTP MCP servers without requiring plaintext secrets in `config.toml`.

## Non-Goals

- SSE transport in this phase.
- MCP server hosting mode (Rikabot as an MCP server).
- UI to manage MCP server config (config file only for now).
- Interactive OAuth/browser credential exchange flows.

## Functional Requirements

### 1) Config model and validation

Add MCP config to `AppConfig`:

- `[mcp]`
  - `enabled` (bool, default `true`)
- `[[mcp.servers]]` list of server definitions

Per-server common fields:

- `name` (required, unique, stable identifier)
- `transport` (`"stdio"` or `"http"`, default `"stdio"`)
- `tool_timeout_secs` (optional, default `180`, max `600`)
- `init_timeout_secs` (optional, default `30`)
- `enabled` (optional, default `true`)

`stdio` fields:

- `command` (required for `stdio`)
- `args` (optional array)
- `env` (optional map)
- `cwd` (optional working directory)

`http` fields:

- `url` (required for `http`, absolute `http/https` URL)
- `headers` (optional map; supports env placeholders in values)

Header secret resolution:

- Header values may contain `${ENV_VAR}` placeholders (for example `Authorization = "Bearer ${LINEAR_MCP_TOKEN}"`).
- Placeholders are resolved at startup using process environment variables.
- If a referenced env var is missing, that MCP server is skipped with a clear error log.
- Secret values must not be printed in logs.

Validation rules:

- duplicate `name` values are rejected at startup
- missing required transport-specific fields are rejected
- invalid transport values are rejected by deserialization
- `tool_timeout_secs` above max is capped to `600`
- `http` transport must accept hosted HTTPS MCP URLs such as Notion and Linear endpoints

### 2) MCP connection lifecycle

At startup:

1. Build base local `ToolRegistry` (existing tools).
2. If MCP is enabled, attempt to connect configured MCP servers in declaration order.
3. For each server:
   - create transport
   - run MCP handshake
   - fetch `tools/list`
   - register wrapped tools into the same registry
4. If one server fails, log error and continue.

Behavior:

- app startup remains successful if at least local tools are available
- MCP connection failure is non-fatal
- connected MCP transports are kept alive for runtime tool calls
- on shutdown, transports/processes are closed best-effort

### 3) MCP protocol flow

Use JSON-RPC 2.0 for both transports.

Connection sequence per server:

1. `initialize`
2. `notifications/initialized` (notification, no result required)
3. `tools/list`

Runtime tool execution:

- call `tools/call` with `{ name, arguments }`
- map result into existing `ToolResult` output string

Compatibility target:

- MCP protocol version string: `2024-11-05`

### 4) Tool naming and registry integration

To avoid name collisions, each discovered MCP tool name is prefixed:

- `<server_name>__<tool_name>`

Example:

- server `filesystem`, tool `read_file` -> `filesystem__read_file`

Wrapper behavior:

- `name()` returns prefixed name
- `description()` uses MCP description or fallback
- `parameters_schema()` uses MCP `inputSchema`
- `execute()` routes to owning MCP server and returns text output/error via existing `ToolResult`

### 5) `stdio` transport requirements

- spawn process with `tokio::process::Command`
- pass configured args/env/cwd
- use piped stdin/stdout, inherited stderr
- write one JSON-RPC request per line
- read one JSON line response per request
- enforce max response line size (e.g., 4 MB)
- enforce per-request timeout
- set `kill_on_drop(true)` so orphan process cleanup is automatic

### 6) `http` transport requirements

- use `reqwest::Client`
- POST JSON-RPC request to configured `url`
- include configured headers on every request after env-placeholder resolution
- require HTTP 2xx status
- parse response body as JSON-RPC response
- apply client timeout and request-level timeout

### 7) Error handling behavior

- handshake failure: server skipped (logged)
- `tools/list` parse failure: server skipped (logged)
- unknown prefixed tool at execution time: return structured tool error
- MCP JSON-RPC error on `tools/call`: return failed tool result with error message
- transport timeout: return failed tool result with timeout message

## Architecture Changes

### New modules

Add MCP modules under `src/tools/`:

- `mcp_protocol.rs`
  - JSON-RPC request/response structs
  - MCP tool definition structs
- `mcp_transport.rs`
  - transport trait
  - `StdioTransport`
  - `HttpTransport`
  - transport factory by config
- `mcp_client.rs`
  - per-server client state
  - initialize/list/call logic
  - multi-server registry + prefixed lookup
- `mcp_tool.rs`
  - `Tool` wrapper for discovered MCP tools

### Existing file updates

- `src/config.rs`
  - add MCP config structs + validation helpers
  - add env-placeholder resolution for MCP HTTP headers
- `src/tools/mod.rs`
  - export/register MCP-related modules
  - add helper to register discovered MCP tools
- `src/main.rs`
  - connect MCP servers during startup
  - inject wrapped MCP tools into the registry before creating `Agent`
- `config.toml`
  - add commented MCP defaults and examples for `stdio` and `http`

## Config Example

```toml
# [mcp]
# enabled = true # default: true

# [[mcp.servers]]
# name = "filesystem"
# transport = "stdio" # default: "stdio"
# command = "npx"
# args = ["-y", "@modelcontextprotocol/server-filesystem", "/tmp"]
# tool_timeout_secs = 180 # default: 180, max: 600
# init_timeout_secs = 30 # default: 30
# enabled = true # default: true

# [[mcp.servers]]
# name = "notion"
# transport = "http" # default: "stdio"
# url = "https://mcp.notion.com/mcp"
# headers = { Authorization = "Bearer ${NOTION_MCP_TOKEN}" }
# tool_timeout_secs = 180 # default: 180, max: 600
# init_timeout_secs = 30 # default: 30
# enabled = true # default: true

# [[mcp.servers]]
# name = "linear"
# transport = "http" # default: "stdio"
# url = "https://mcp.linear.app/mcp"
# headers = { Authorization = "Bearer ${LINEAR_MCP_TOKEN}" }
# tool_timeout_secs = 180 # default: 180, max: 600
# init_timeout_secs = 30 # default: 30
# enabled = true # default: true
```

Notes:

- Header env placeholders use `${ENV_VAR}` syntax and are resolved once at startup.
- Header values after resolution are only kept in memory and are not written back to config files.

## Test Requirements

Add focused tests for non-trivial behavior:

1. Config parsing/validation
   - valid `stdio` and `http` blocks parse correctly
   - missing `command` for `stdio` fails
   - missing `url` for `http` fails
   - duplicate server names fail
   - header env placeholders resolve correctly
   - missing header env var causes per-server failure (non-fatal globally)
2. MCP protocol and tool wrapper
   - prefixed name mapping is deterministic
   - wrapper forwards schema/description correctly
3. Transport behavior
   - `stdio` spawn failure returns clean error
   - `http` transport requires URL and handles non-2xx responses
4. Registry behavior
   - failed server connection does not abort connecting others
   - unknown prefixed tool returns clear error

### MCP spec compliance tests (required)

Add protocol-level tests to verify behavior against official MCP/JSON-RPC expectations:

1. JSON-RPC envelope correctness
   - all outbound requests use `jsonrpc = "2.0"`
   - call requests include `id`, notifications omit `id`
   - response parsing accepts `result` and `error` shapes per JSON-RPC 2.0
2. MCP initialize handshake contract
   - client sends `initialize` first with declared protocol version (`2024-11-05`)
   - client sends `notifications/initialized` only after successful initialize response
   - initialize error response is surfaced and server is skipped (non-fatal globally)
3. MCP tool discovery contract
   - `tools/list` response decoding validates tool `name` and `inputSchema`
   - malformed `tools/list` payload causes per-server failure with clear error log
4. MCP tool call contract
   - `tools/call` request payload uses `{ name, arguments }`
   - successful result is propagated as tool output
   - JSON-RPC error object from MCP server maps to failed `ToolResult`
5. Official sample flow coverage
   - include mock-server test fixtures that emulate the official flow:
     `initialize -> notifications/initialized -> tools/list -> tools/call`
   - run this flow for both `stdio` and `http` transports

## Migration and Compatibility

- Existing local tools remain unchanged.
- MCP is additive and opt-in through config.
- Existing provider/tool-call loop protocol remains unchanged.
- If MCP config is absent, behavior is identical to current release.

## Implementation Plan

### Phase 1: config + protocol + transports

1. Add MCP config structs and validation in `src/config.rs`.
2. Add `mcp_protocol` and `mcp_transport` modules with unit tests.

### Phase 2: client + tool wrapping

1. Add MCP client/registry module for initialize/list/call flow.
2. Add MCP tool wrapper implementing existing `Tool` trait.

### Phase 3: wiring + docs

1. Wire MCP connection and registration in `main`/`tools` initialization path.
2. Add config example comments to `config.toml`.
3. Add/adjust tests to cover failure isolation and end-to-end registration.
4. Add MCP spec-compliance tests using mock MCP servers for both transports.

## Decisions Applied

1. Support `stdio` and `http` transports in this PRD; defer SSE.
2. Prefix MCP tool names with `<server>__<tool>` to prevent collisions.
3. Treat per-server MCP connection failures as non-fatal.

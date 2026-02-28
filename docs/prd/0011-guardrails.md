# Tool Permissions Policy (Backend + Web)

## Context

Rikabot currently executes any registered tool call with no policy gate. This includes:

- local tools (`shell`, `filesystem_*`)
- dynamically discovered MCP tools (prefixed as `<server>__<tool>`)

We need first-class tool permissions policy with a Claude Code-like wildcard rule style for tool arguments, plus a web UI section that can edit policy and persist it to `config.toml`.

Reference direction:

- Claude Code permissions model (allow/deny rules with wildcard argument matching): <https://code.claude.com/docs/en/permissions>
- existing Rikabot architecture:
  - config load: `src/config.rs`
  - tool execution boundary: `src/tools/mod.rs` + `src/agent/mod.rs`
  - websocket gateway: `src/gateway/ws.rs`
  - web UI: `web/src/App.svelte`

## Goals

1. Add configurable tool permission policy with `allow` + `deny` rules.
2. Support wildcard matching for tool arguments in a Claude-like expression format.
3. Enforce policy at runtime for every tool execution.
4. Ensure enforcement also works for dynamically changing tool sets (for example MCP tools added/removed while runtime is alive).
5. Add a web UI section to view/edit/save permissions.
6. Persist permission configuration in `config.toml` (single source of truth).
7. Apply policy updates at runtime without server restart.

## Non-Goals

- Interactive user approval prompts in this phase (future extension).
- Full ACL/role system by user/session/channel.
- Sandboxing/process isolation redesign.
- Per-channel or per-thread permission overrides in this phase.

## Policy Model

### Config shape

Add to `AppConfig`:

```toml
[permissions]
enabled = true

[permissions.tools]
allow = [
  "shell(command:npm run *)",
  "shell(command:git commit *)",
  "shell(command:git * main)",
  "shell(command:* --version)",
  "shell(command:* --help *)"
]
deny = [
  "shell(command:git push *)"
]
```

Notes:

- `permissions.enabled` default: `true`
- if allow list is empty, execution is denied by default
- deny rules always take precedence over allow rules

### Rule expression format

Rule grammar (v1):

- `<ToolPattern>(<ArgPattern>)`

Examples:

- `shell(command:git commit *)`
- `shell(git commit *)`
- `filesystem_read(*)`
- `filesystem_read(path:docs/*)`

Normalization:

- tool pattern matching is case-insensitive
- argument matching is case-sensitive (raw command/text semantics preserved)
- surrounding whitespace is trimmed

Wildcard semantics:

- `*` matches zero or more characters

## Matching and Evaluation Semantics

For a tool invocation `(tool_name, args_json)`:

1. Build canonical argument text:
   - `shell`: use `args.command` string
   - other tools: use compact stable JSON serialization of `args`
2. Evaluate deny rules first:
   - if any deny rule matches, block execution
3. Evaluate allow rules:
   - if allow list is empty: deny (default deny)
   - if allow list is non-empty: require at least one allow match
4. If blocked, return tool error result (do not run underlying tool)

Blocking result contract:

- `ToolResult.success = false`
- `ToolResult.error` includes reason and matched rule when available

## Dynamic Tool Handling Requirement

Policy must not rely on static tool registration snapshots.

Enforcement must happen at execution time (inside `ToolRegistry::execute`) using the invoked `name` and `args`. This guarantees correct behavior when tools change during runtime (for example MCP reconnect/discovery changes).

## Architecture Changes

### New module

Add `src/permissions/mod.rs`:

- config structs:
  - `PermissionsConfig`
  - `ToolPermissionsConfig`
- compiled policy types:
  - `CompiledRule`
  - `PermissionEngine`
- parser/compiler:
  - parse `<tool>(<arg>)` expressions
  - compile wildcard patterns to regex
- evaluator:
  - `evaluate(tool_name, args_json) -> PermissionDecision`

### Tool execution integration

Update `src/tools/mod.rs`:

- add shared `Arc<RwLock<PermissionEngine>>` into `ToolRegistry`
- check permissions inside `ToolRegistry::execute` before dispatching tool
- expose method to hot-swap/refresh policy

### Config load + persistence plumbing

Update `src/config.rs`:

- add permissions config structs and validation
- preserve defaults and backward compatibility when section is absent

Add config persistence helper (new module, for example `src/config_store.rs`):

- read current `config.toml`
- patch only `[permissions]` / `[permissions.tools]`
- write atomically
- preserve unrelated sections/comments via `toml_edit` (recommended over full serde rewrite)

### Runtime shared state

Update `src/gateway/mod.rs::AppState`:

- add policy state handle (`Arc<RwLock<PermissionEngine>>`)
- add config-store handle for persistence writes

### WebSocket protocol additions

Client -> server:

- `permissions_get`
- `permissions_set` with payload:
  - `enabled: bool`
  - `allow: string[]`
  - `deny: string[]`

Server -> client:

- `permissions_state` (current config + validation warnings)
- `permissions_updated` (ack with effective config)
- `error` (existing channel) for validation/persist failures

## Web UI Requirements

Add a new Permissions section in `web/src/App.svelte` (sidebar or settings panel):

1. Read current permission config on connect (`permissions_get`).
2. Show:
   - enabled toggle
   - allow rules editable as line-based list
   - deny rules editable as line-based list
3. Save action:
   - sends `permissions_set`
   - disabled while a save is in flight
4. Feedback:
   - inline validation errors from server
   - last-saved confirmation state
5. Changes apply live to subsequent tool calls after successful save.
6. Cmd/Ctrl+K command palette includes navigation to the settings/permissions panel.

Persistence contract:

- UI edits always persist into `config.toml`.
- in-memory policy is updated only after successful persistence and validation.

## Validation Rules

1. Rule must match `<tool>(<args>)` shape.
2. Tool and arg patterns cannot be empty.
3. Maximum rule count per list (for example 500) to prevent pathological config.
4. Maximum rule length (for example 1,024 chars).
5. Invalid regex compilation from wildcard transform must fail fast.

## Observability

Add structured logs:

- policy loaded/updated (count of allow/deny rules, not full sensitive args)
- blocked invocation with tool name + matched rule id
- config update success/failure

Add counters:

- `permissions.blocked_total`
- `permissions.allowed_total`
- `permissions.config_update_total{status=...}`

## Security and Behavior Notes

- permissions policy is hard enforcement, unlike prompt-only safety text
- deny-first precedence prevents accidental broad allowlist bypasses
- default-deny when allowlist is empty enforces explicit policy
- rule evaluation should be deterministic and side-effect free

## Test Requirements

Add focused tests for non-trivial behavior:

1. Rule parsing/compilation:
   - valid and invalid expression cases
   - wildcard matching behavior
2. Decision precedence:
   - deny over allow
   - empty allow means allow-unless-deny
3. Canonical argument extraction:
   - `shell` uses command string
   - other tools use stable JSON
4. Tool execution integration:
   - blocked tools do not execute underlying implementation
5. Runtime update flow:
   - `permissions_set` updates in-memory policy and persists
   - invalid updates do not mutate active policy
6. Config persistence:
   - writes `[permissions]` fields correctly
   - preserves unrelated `config.toml` content structure/comments when using `toml_edit`
7. Web UI behavior:
   - renders permissions section
   - sends correct websocket payload on save
   - reflects server-side validation errors

## Implementation Plan

### Phase 1: policy engine + enforcement

1. Add `permissions` module with parser, compiler, evaluator.
2. Extend config model and startup loading.
3. Inject policy engine into `ToolRegistry` and enforce in `execute`.
4. Add unit tests for parser and precedence.

### Phase 2: runtime config updates + persistence

1. Add config store module for patching `config.toml`.
2. Extend `AppState` with policy/config handles.
3. Add websocket `permissions_get` / `permissions_set`.
4. Add integration tests for update success/failure paths.

### Phase 3: web UI editing

1. Add Permissions UI section in `App.svelte`.
2. Add fetch/save flows and inline error handling.
3. Add frontend tests (or high-confidence interaction tests) for protocol wiring.

### Phase 4: hardening

1. Add observability logs/counters.
2. Add rule-count/length safeguards.
3. Document config examples and troubleshooting.

## Acceptance Criteria

1. Tool calls are blocked/allowed according to configured wildcard rules.
2. Deny rules override allow rules.
3. A rule such as `shell(command:git push *)` blocks matching shell calls.
4. Permissions can be edited from web UI and saved successfully.
5. Saved permissions persist in `config.toml`.
6. Updated policy takes effect immediately for new tool invocations without restart.
7. Dynamic tool names are evaluated correctly at call time.

## Decisions

1. Tool names are exact-match only in v1 (no tool-name aliases).
2. Wildcard support in v1 is `*` only.
3. Structured selectors are required (for example `filesystem_read(path:docs/*)`).
4. Default policy is deny unless an allow rule matches.

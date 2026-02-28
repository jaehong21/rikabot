# Ask-for-Approval on Permission Deny + MCP Tool Namespace Update

## Context

Rikabot currently enforces permission rules at tool execution time, but a permission block is surfaced as a generic failure:

- backend returns `ToolResult.success = false`
- UI renders tool status as `Failed`
- logs do not clearly separate `permission denied` from execution/runtime failures

Additionally:

- there is no interactive "ask for approval" flow when a tool is blocked
- MCP tool names are currently `<server>__<tool>`, which does not explicitly identify them as MCP tools
- permission rules currently require exact tool names; tool-name wildcards (for example `mcp_linear_*`) are not supported

## Current-State Verification (2026-02-28)

Verified from code:

1. Permission deny path returns generic failure:
   - `src/tools/mod.rs` blocks and returns `ToolResult { success: false, ... }`
2. UI only has `running | success | failure` tool status:
   - `web/src/App.svelte` `ToolEntry.status`
   - `statusText()` maps non-success to `Failed`
3. Tool-name wildcard is explicitly rejected:
   - `src/permissions/mod.rs` rejects `*` in tool name
4. MCP tool prefix is currently `<server>__<tool>`:
   - `src/tools/mcp_client.rs`

Conclusion:

- Requested `mcp_linear_*` allow pattern is **not possible** in current implementation.
- Interactive approval on deny is **not implemented**.

## Goals

1. Distinguish permission block (`Denied`) from execution failure (`Failed`) in backend events, UI, and logs.
2. Add interactive approval flow when a tool is blocked by permissions.
3. Provide three approval options:
   - allow persistently (save allow rule in `config.toml`)
   - allow once (single invocation only)
   - deny
4. Suggest an allow rule with wildcard by default for "allow persistently".
5. Rename MCP tool namespace from `<server>__<tool>` to `mcp_<server>__<tool>`.
6. Support tool-name wildcard matching so patterns like `mcp_linear_*` can allow all Linear MCP tools.
7. Keep existing deny-first precedence and default-deny behavior.

## Non-Goals

- Full role-based approvals or multi-user permission policies.
- Per-thread policy files or separate config stores.
- Replacing existing permission grammar entirely.

## Functional Requirements

### 1) Tool Result Classification

Introduce explicit tool result status:

- `running`
- `success`
- `denied`
- `failed`

Rules:

- permission engine block => `denied`
- tool runtime error or unknown tool => `failed`
- successful execution => `success`

`denied` must not be counted as generic failure in UI labels.

### 2) Logging and Metrics for Deny

Add explicit deny logs:

- tool name
- normalized args summary (safe/redacted as needed)
- matched deny reason/rule (if available)
- run/session identifiers when available

Add counters:

- `tool_call_denied_total`
- `tool_call_failed_total`
- `tool_call_success_total`

### 3) Ask-for-Approval Flow

When permission denies a tool call:

1. Emit tool result as `denied`.
2. Emit approval request event to UI with:
   - `request_id`
   - `tool_name`
   - `args`
   - `deny_reason`
   - `suggested_allow_rule`
3. Pause tool execution for that call until user decision.
4. User chooses one:
   - `allow_persist`
   - `allow_once`
   - `deny`
5. Resume:
   - `allow_persist`: validate + persist rule + reload in-memory policy + retry same tool call
   - `allow_once`: retry same tool call once without persistence
   - `deny`: keep denied result and continue agent loop

If no decision is received (disconnect/timeout), treat as `deny`.

### 4) Suggested Allow Rule

Generate a recommended rule per blocked call:

- shell example: `shell(command:git commit *)`
- filesystem read example: `filesystem_read(path:workspace/*)`
- MCP server-wide example: `mcp_linear_*(*)`

For MCP tools, default recommendation for persistent allow should be server-scoped wildcard:

- `mcp_<server>_*(*)`

User can edit before confirming persistent save.

### 5) MCP Tool Naming

Change MCP tool names to:

- `mcp_<server>__<tool>`

Examples:

- `linear__search_issues` -> `mcp_linear__search_issues`
- `notion__query` -> `mcp_notion__query`

Purpose:

- explicit distinction between local and MCP tools
- enable concise MCP-scoped allow patterns

### 6) Tool Pattern Wildcard in Permissions

Extend rule parser to allow wildcard in tool pattern.

Current:

- exact tool name only

New:

- `*` allowed in tool pattern
- case-insensitive match remains

Examples:

- `mcp_linear_*(*)` => allow all tools from Linear MCP
- `mcp_*(*)` => allow all MCP tools
- `filesystem_*(*)` => allow all filesystem local tools

Deny precedence remains unchanged.

## Policy Model Update

Rule grammar (v2):

- `<ToolPattern>(<ArgPattern>)`

Where:

- `ToolPattern`: exact or wildcard string (`*` matches zero or more chars)
- `ArgPattern`: existing behavior (`*`, raw wildcard, or selector mode `path:pattern`)

Examples:

- `shell(command:*cat *)`
- `filesystem_read(path:docs/*)`
- `mcp_linear_*(*)`

## Protocol Changes (WebSocket)

### Server -> Client

- `tool_call_result`
  - add `status: "success" | "failed" | "denied"` (keep `success` for backward compatibility during transition)
- `tool_approval_required`
  - `request_id`
  - `tool_name`
  - `args`
  - `deny_reason`
  - `suggested_allow_rule`

### Client -> Server

- `tool_approval_decision`
  - `request_id`
  - `decision: "allow_persist" | "allow_once" | "deny"`
  - `allow_rule?` (required for `allow_persist`, optional override)

## UI Requirements

In tool block UI:

1. Show `Denied` badge for permission blocks (not `Failed`).
2. On denied status, show approval panel with three actions:
   - `Always allow (save)`
   - `Allow once`
   - `Deny`
3. Pre-fill editable allow rule with server-provided `suggested_allow_rule`.
4. While waiting, show pending state and disable duplicate submissions.
5. Reflect final status after decision and retry outcome.

Thread-level stats update:

- track/display denied separately from failed.

## Backend Architecture Changes

### 1) Permission Decision and Tool Result Types

Update execution contract so caller can distinguish:

- policy deny
- execution failure
- success

Likely touch points:

- `src/permissions/mod.rs`
- `src/tools/mod.rs`
- `src/agent/mod.rs`
- `src/gateway/ws.rs`

### 2) Approval Coordination

Add an approval coordinator for active runs:

- create approval request
- publish `tool_approval_required`
- await user decision
- resume blocked tool call deterministically

This must coexist with:

- existing single active run model
- kill switch
- websocket reconnect replay behavior

### 3) Persist-on-Approve Path

For `allow_persist`:

1. validate rule
2. append/dedupe in `permissions.tools.allow`
3. persist using existing config store
4. hot-reload permission engine
5. retry blocked call

## Compatibility and Migration

### MCP name migration

Because tool names change, existing MCP-specific allow/deny rules in `config.toml` may stop matching.

Migration expectation:

- update rules from `<server>__...` to `mcp_<server>__...` or wildcard forms like `mcp_<server>_*(*)`

Optional implementation enhancement:

- one-time migration helper for known MCP server names (nice-to-have, not required for v1 of this PRD)

### Event compatibility

Keep `success` boolean in `tool_call_result` for one transition phase, but UI should switch to `status` first.

## Security and Safety Notes

1. Persistent allow writes must require explicit user action per denied request.
2. Suggested rules should prefer least privilege feasible for the current call.
3. `allow_once` must never mutate config.
4. Approval decisions must be bound to `request_id` and active run to prevent stale or cross-run decisions.

## Test Requirements

1. Permission parser:
   - tool-name wildcard valid/invalid cases
   - matching behavior for `mcp_linear_*`
2. MCP naming:
   - registry generates `mcp_<server>__<tool>`
3. Denied classification:
   - permission block maps to `denied`, not `failed`
4. Approval flow:
   - allow persist => config updated + policy reloaded + tool retried
   - allow once => tool retried, config unchanged
   - deny => no retry
   - timeout/disconnect => deny
5. UI:
   - denied badge renders
   - approval actions send correct websocket payload
   - counters separate denied vs failed
6. Logging/metrics:
   - denied events and counters emitted distinctly

## Implementation Plan

### Phase 1: Core status and parser updates

1. Add explicit tool result status (`denied`).
2. Extend permission parser to support tool-name wildcard.
3. Rename MCP tool names to `mcp_<server>__<tool>`.

### Phase 2: Approval runtime

1. Add websocket events/messages for approval request/decision.
2. Add approval coordinator in run pipeline.
3. Implement retry logic for `allow_once` and `allow_persist`.

### Phase 3: UI integration

1. Add denied badge and approval panel.
2. Add editable suggested rule field.
3. Add separate denied stats display.

### Phase 4: Hardening

1. Add migration notes/docs for MCP naming change.
2. Add structured logs and counters.
3. Complete integration and reconnect/kill-switch edge-case tests.

## Acceptance Criteria

1. Permission-blocked tool calls are rendered as `Denied`, not `Failed`.
2. Denied tool calls produce explicit deny logs and counters.
3. UI prompts user with three options: persist allow, allow once, deny.
4. `allow_persist` updates `config.toml` and applies immediately.
5. `allow_once` allows only the current blocked call and does not persist.
6. MCP tool names use `mcp_<server>__<tool>`.
7. Rule `mcp_linear_*(*)` correctly allows all tools from `linear` MCP server.

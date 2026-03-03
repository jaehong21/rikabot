# Settings Section Deep-Linking and Skills/MCP Visibility

## Context

- The current settings UI stores the selected section (`general`, `permissions`, `skills`, `mcp`) only in local component state (`web/src/routes/settings-page.tsx`), so refresh/navigation resets to `general`.
- The current Skills section is static copy only. It does not show which skills are loaded, their availability, or source paths.
- The current MCP section shows server readiness and tool counts from `mcp_status`, but does not show supported tool names.
- User request scope:
  - Deep-link section state via query string (e.g. `/settings?section=skills`) so refresh stays on the selected section.
  - Show loaded skills and provide editing capability.
  - Show supported tools per MCP server.

## Goals

1. Add URL-driven section state for settings page via `section` query parameter.
2. Add Skills visibility in settings showing currently loaded skills, availability, and source file path.
3. Add a safe skill editing flow from settings for workspace `SKILL.md` files.
4. Add MCP tool visibility in settings showing per-server supported tool names (not only counts).
5. Preserve backward compatibility with existing settings/mcp behavior when new metadata is unavailable.

## Non-Goals

1. Building a full IDE/editor experience for skills (syntax highlight, linting, preview diff).
2. Supporting edit operations outside workspace skill files (`<workspace>/skills/*/SKILL.md`).
3. Managing MCP server config lifecycle (add/remove servers) from the settings UI.
4. Introducing database persistence for settings section state (URL query string is the source of truth).
5. Editing AGENTS.md or non-skill prompt bootstrap files in this phase.

## Requirements

### Functional Requirements

1. Settings section query parameter:

- Route must support `section` query values: `general`, `permissions`, `skills`, `mcp`.
- Canonical deep-link format: `/settings?section=<id>`.
- `section` query is only supported on `/settings`; `/?section=...` is out of scope.
- On initial load:
  - valid `section` query selects that section.
  - missing/invalid value falls back to `general`.
- Clicking a section in sidebar updates the query parameter immediately.
- Refreshing page must preserve selected section via URL.
- Browser back/forward must restore section according to URL history.

2. Skills status visibility:

- Settings/Skills must render live skills snapshot from backend instead of static placeholder copy.
- Each skill row shows:
  - `name`
  - `description`
  - `available` status
  - `always` flag (always-loaded vs on-demand)
  - source path
  - missing requirements (if unavailable)
- Empty and disabled states must be explicit (e.g. "Skills disabled", "No skills found").

3. Skills edit flow:

- User can open a selected skill and edit full `SKILL.md` content in settings.
- Save action writes to the same skill file atomically.
- Backend validation must reject writes when:
  - target path is outside workspace skills directory
  - file is not named `SKILL.md`
  - content is empty or invalid frontmatter for required fields (`name`, `description`)
- On successful save:
  - UI shows success state.
  - skills snapshot refreshes and reflects latest metadata.

4. MCP tool visibility:

- MCP section must show per-server tool list for servers in `ready` state.
- For each tool, show at minimum:
  - tool name (human/original MCP name only)
  - optional description when available
- Server cards keep existing state labels (`Ready`, `Connecting`, `Failed`, `Disabled`).
- If server is not ready, tool list area shows error/empty state text.

5. WebSocket protocol updates:

- Add event payload for skills snapshot (e.g. `skills_status`).
- Extend `mcp_status` payload to include tool metadata list per server.
- Send initial snapshots on websocket connect and refresh on relevant changes.

### Non-Functional Requirements

1. Compatibility:

- Frontend must remain functional if backend sends legacy `mcp_status` without tools list.
- Unknown query params must not break routing.

2. Safety:

- Skill edit API must enforce workspace path boundary checks and atomic writes.
- Error payloads must avoid leaking secrets or unrelated filesystem paths.

3. Performance:

- Settings page render should remain responsive with up to 250 MCP tools total and 100 skills.
- WebSocket payload size for skills/MCP status should remain bounded (avoid full schema blobs).

4. Reliability:

- MCP reconnects must update tool list snapshot consistently after state changes.
- Failed skill save must not partially overwrite files.

## Architecture and Design Impact

1. Frontend route state:

- `settings-page.tsx` section state moves from local-only `useState` to URL-backed state (`section` query).
- `web/src/router.tsx` settings route defines/validates query shape.

2. Frontend data model:

- `web/src/types/app.ts` adds `SkillStatus` and websocket event types for skills snapshot.
- `web/src/context/app-store.tsx` stores skills list + edit lifecycle state (loading/saving/errors).

3. Backend websocket contract:

- `src/gateway/ws.rs` adds client messages for skills fetch/edit and server event for skills snapshot.
- Existing `mcp_status` payload extended with optional `tools` list per server.

4. Skills backend surface:

- `src/skills/mod.rs` (or dedicated helper module) exposes serializable snapshot/read/write helpers for workspace skills.
- `src/prompt/mod.rs` exposes workspace path access for gateway handlers (or equivalent dependency injection).

5. MCP runtime enrichment:

- `src/mcp_runtime.rs` status model includes `tools: Vec<McpToolStatus>` populated on successful connect.
- Connection/retry logic resets and republishes tool list on state transitions.

6. Tradeoff:

- Reusing websocket keeps transport simple but increases event union complexity; mitigated by optional fields + strict typing in `web/src/types/app.ts`.

## Implementation Plan

### Phase 1: Setup

1. Add query-state model for settings route and section enum normalization.
2. Add/extend shared frontend types for `section`, skills payload, and MCP tool metadata.
3. Add backend serializable DTOs for skills and MCP tool status snapshots.
4. Define websocket message names and payload contracts in code comments and tests.

### Phase 2: Core Work

1. Implement URL-driven settings navigation:

- read query on mount
- update query when sidebar section changes
- preserve behavior on refresh/back/forward.

2. Implement Skills snapshot flow:

- backend: load workspace skills via `SkillsLoader`-based API and send `skills_status`.
- frontend: render skills list, unavailable reasons, empty/disabled states.

3. Implement Skills edit flow:

- frontend: open editor for selected skill, submit save, display inline errors/success.
- backend: validate target path/content and perform atomic write.

4. Implement MCP tool list flow:

- backend: capture tool metadata at MCP connect time and include it in `mcp_status`.
- frontend: render per-server tool lists in MCP section with fallback states.

### Phase 3: Validation

1. Add Rust unit tests for skills serialization, path guardrails, and MCP snapshot tool metadata.
2. Add/extend frontend typecheck and behavior checks for query-based section state.
3. Manual smoke tests:

- deep-link load/refresh/back-forward
- skill list visibility and edit save/reload
- MCP tools visibility for ready and failed servers.

4. Rollback/compatibility check:

- verify frontend still works against backend that sends only legacy `mcp_status` fields.
- verify removing `section` query still defaults to `general`.

## File-by-File Changes

1. `web/src/router.tsx`

- add settings route search typing/validation for `section`.

2. `web/src/routes/settings-page.tsx`

- replace local-only section state with URL query sync.
- replace static Skills copy with dynamic list + edit UI.
- extend MCP section UI to show per-server tools.

3. `web/src/context/app-store.tsx`

- add skills state/actions (`refreshSkills`, `saveSkill`).
- process new websocket events/messages for skills and MCP tool metadata.

4. `web/src/types/app.ts`

- add `SkillStatus`, `SkillsStatusSnapshot`, `McpToolStatus` and updated event unions.

5. `src/gateway/ws.rs`

- add handlers for `skills_get` and `skills_set`.
- send initial and updated `skills_status`.
- extend `mcp_status` serialization to include optional tools list.

6. `src/gateway/mod.rs`

- extend `AppState` dependencies if needed for skill file update/read operations.

7. `src/skills/mod.rs`

- expose reusable skill snapshot/read/write helpers and validation.

8. `src/prompt/mod.rs`

- expose workspace path accessor or helper required by gateway skill handlers.

9. `src/mcp_runtime.rs`

- include per-server tool metadata in runtime snapshot and state transitions.

10. `src/tools/mcp_client.rs`

- expose helper(s) needed to map connected server tool defs into runtime status.

## Testing and Validation

1. Backend tests (`cargo test`):

- skills snapshot returns expected fields for available/unavailable skills.
- skill edit rejects non-`SKILL.md` paths and path traversal attempts.
- skill edit persists valid content atomically.
- MCP runtime snapshot includes tool list on ready state and clears list on failure/retry.

2. Frontend checks:

- `cd web && bun run typecheck`.
- verify compile path for new route search typing and event unions.

3. Manual scenario checks:

- open `/settings?section=skills`, refresh, confirm Skills remains active.
- navigate among sections and confirm URL updates.
- edit a skill description, save, refresh, confirm updated metadata appears.
- verify MCP section shows tool names for ready servers and fallback message for failed servers.

4. Compatibility check:

- run frontend against payload without `tools` list and confirm no runtime errors.

## Acceptance Criteria

1. Visiting `/settings?section=skills` opens Settings with Skills section selected, and refresh keeps it selected.
2. Invalid query values (e.g. `/settings?section=unknown`) resolve to `general` without rendering errors.
3. Skills section shows real loaded skills from workspace with name, availability, and path metadata.
4. Editing a valid `SKILL.md` from settings persists changes and updates the rendered skill metadata without app restart.
5. Attempts to edit outside `<workspace>/skills/**/SKILL.md` are rejected with a visible error and no file modification.
6. MCP section shows each ready server’s tool names (and descriptions when present), not just aggregate count.
7. Existing behavior for permissions/general settings remains unchanged.
8. Backward compatibility check passes: UI works when backend omits new optional metadata fields.

## Risks and Mitigations

1. **Risk**: Skill editing introduces arbitrary file write vulnerability.
   - **Impact**: High (security and integrity).
   - **Mitigation**: Canonical path checks, strict filename constraint (`SKILL.md`), workspace boundary enforcement, atomic writes.
2. **Risk**: MCP tool metadata may become stale after reconnect/failure transitions.
   - **Impact**: Medium (misleading UI).
   - **Mitigation**: Reset tools list on non-ready states and republish snapshot on each status transition.
3. **Risk**: Query-param state and local state drift causes inconsistent section selection.
   - **Impact**: Medium (navigation bugs).
   - **Mitigation**: Make URL query the single source of truth for selected section.
4. **Risk**: Large MCP tool lists degrade render performance.
   - **Impact**: Low/Medium.
   - **Mitigation**: Render compact rows, avoid schema payloads, and keep metadata fields minimal.

## Decisions Applied

1. Skills editing supports full-file `SKILL.md` editing (frontmatter + body).
2. Section deep-links are accepted only on `/settings?section=...`.
3. MCP section displays original tool names only per server context.

# WebSocket to REST CRUD Migration

## Context

- The frontend currently uses WebSocket for all backend communication, including non-streaming CRUD and settings operations.
- In `src/gateway/ws.rs`, many inbound message types are request/response style (`thread_*`, `permissions_*`, `skills_*`) and do not require full-duplex streaming.
- `web/src/routes/chat-page.tsx` still needs real-time streaming and two-way control, so chat generation and run control should stay on WebSocket.
- This migration is a hard cutover for CRUD/settings: once REST endpoints are added, migrated WebSocket handlers are removed in the same change.
- Existing automated coverage includes backend tests and Playwright E2E specs under `tests/e2e/specs`, so both layers must be updated as part of this migration.

## Goals

1. Migrate CRUD/settings operations from WebSocket messages to REST endpoints.
2. Keep chat runtime interactions (`message`, stream events, `kill_switch`, approvals) on WebSocket.
3. Use `axios` via a shared `axiosInstance` with `baseURL` for frontend REST calls.
4. Remove migrated WebSocket handlers immediately (no legacy compatibility mode).
5. Update backend and E2E test suites so the migrated behavior is fully validated.

## Non-Goals

1. Replace chat streaming (`message` -> `chunk`/`done`) with REST.
2. Add REST duplicates for `kill_switch`, tool approval decisions, or `mcp_status` push events.
3. Introduce auth/authorization redesign in this PRD.
4. Support dual transport paths for migrated CRUD operations after merge.
5. Implement future multi-session concurrent run orchestration.

## Requirements

### Functional Requirements

1. Implement REST endpoints for migrated CRUD/settings operations:

- `GET /api/threads`
- `POST /api/threads`
- `PATCH /api/threads/:session_id`
- `GET /api/threads/:session_id/messages`
- `DELETE /api/threads/:session_id/messages`
- `DELETE /api/threads/:session_id`
- `GET /api/settings/permissions`
- `PUT /api/settings/permissions`
- `GET /api/settings/skills`
- `GET /api/settings/skills/content?path=...`
- `PUT /api/settings/skills/content`

2. Keep these operations/events WebSocket-only:

- inbound: `message`, `kill_switch`, `tool_approval_decision`
- outbound: `user_message`, `chunk`, `tool_call_start`, `tool_call_result`, `tool_approval_required`, `done`, `stopped`, `mcp_status`

3. Frontend transport migration in `web/src/context/app-store.tsx`:

- Replace CRUD/settings `sendControl(...)` usage with `axiosInstance` calls.
- Add a shared `axiosInstance` configured with backend HTTP `baseURL` for dev/prod environments.
- Keep existing UI behavior and slash command behavior unchanged while only changing transport.

4. Remove migrated WebSocket handlers in the same change:

- Remove WS inbound CRUD/settings message handling for:
  - `thread_list`, `thread_create`, `thread_rename`, `thread_switch`, `thread_clear`, `thread_delete`
  - `permissions_get`, `permissions_set`
  - `skills_get`, `skills_read`, `skills_set`
- Remove frontend reliance on corresponding WS CRUD/settings events.
- Do not ship legacy compatibility paths for these migrated operations.

5. Current session ownership moves to client-local state:

- Frontend owns selected thread/session in app state/router.
- Backend focuses on thread list and per-session history retrieval.
- No new API contract should require server-global current session semantics.

6. Response and error contract:

- REST responses must provide data needed by current reducers (sessions, history, permissions, skills).
- REST failures return consistent JSON error payloads with proper HTTP status codes.

### Non-Functional Requirements

1. Reliability:

- CRUD/settings operations must work without an active WebSocket.
- Chat runtime WebSocket behavior must remain stable during reconnects.

2. Performance:

- P95 latency target for thread/settings endpoints: <= 300ms in local/dev.
- No measurable regression to chat first-token streaming latency.

3. Operability:

- Keep `/health` unchanged.
- Log REST method/path/status/duration for debugging migration issues.

4. Release coordination:

- Because this is a hard cutover, frontend and backend changes must ship together.

## Architecture and Design Impact

1. Gateway routing:

- Extend `src/gateway/mod.rs` to register REST routes under `/api/...` while keeping `/ws` for runtime communication.

2. REST handlers:

- Add `src/gateway/rest.rs` for thread/settings REST handlers.
- Reuse core logic from `src/session/mod.rs`, `src/config_store.rs`, and `src/skills/mod.rs`.

3. WebSocket scope reduction:

- `src/gateway/ws.rs` remains focused on chat/run lifecycle and push-style runtime events.
- CRUD/settings message handlers listed above are removed.

4. Frontend HTTP client:

- Add an axios client module (for example `web/src/lib/api/axios.ts`) exporting `axiosInstance` with environment-aware `baseURL`.
- App store methods call REST through this shared instance.

5. Session model direction:

- Session/thread selection is client-local.
- API contracts should not depend on returning or mutating server-global `current_session_id`.

## Implementation Plan

### Phase 1: Setup

1. Add REST router scaffolding and request/response DTOs.
2. Add shared frontend `axiosInstance` with `baseURL` configuration.
3. Refactor reusable CRUD/settings logic out of WS-only code paths where needed.
4. Define and document hard-cutover list of WS handlers to remove.

### Phase 2: Core Work

1. Implement thread REST endpoints and wire to `SessionManager`.
2. Implement permissions/skills REST endpoints and wire to existing persistence and validation logic.
3. Update frontend app-store CRUD/settings actions to use `axiosInstance`.
4. Remove migrated WS CRUD/settings handlers and associated frontend event dependencies.
5. Keep WebSocket chat runtime flows intact (`message`, stream events, `kill_switch`, approvals, `mcp_status`).

### Phase 3: Validation

1. Add/adjust backend tests for all new REST endpoint families.
2. Update and run Playwright E2E suite to validate migrated CRUD/settings behavior via UI flows.
3. Re-run chat/runtime E2E coverage to confirm WebSocket-only flows still pass.
4. Add a rollback check procedure (revert this migration patchset and confirm baseline test commands still pass).

## File-by-File Changes

1. `src/gateway/mod.rs`

- Register `/api` routes and compose REST handler module.

2. `src/gateway/rest.rs` (new)

- Implement REST handlers, DTOs, and HTTP error mapping.

3. `src/gateway/ws.rs`

- Remove migrated CRUD/settings WS handlers.
- Keep runtime WebSocket handlers/events only.

4. `src/session/mod.rs`

- Reuse existing session operations; add small helpers only if required for REST responses.

5. `src/config_store.rs`

- Continue permissions persistence for REST update path.

6. `src/skills/mod.rs`

- Reuse skills snapshot/read/write functions in REST handlers.

7. `web/src/lib/api/axios.ts` (new)

- Export `axiosInstance` with backend `baseURL` configuration.

8. `web/src/context/app-store.tsx`

- Replace CRUD/settings WebSocket control sends with axios REST calls.
- Keep runtime WebSocket connect and event handling for chat/approval/status flows.

9. `web/src/types/app.ts`

- Remove migrated WS CRUD/settings event dependencies.
- Add REST DTO typing used by app store.

10. `tests/e2e/specs/thread-lifecycle.spec.ts`

- Update assertions to validate the REST-backed thread lifecycle behavior through UI.

11. `tests/e2e/specs/slash-commands.spec.ts` and `tests/e2e/specs/threads-explorer.spec.ts`

- Update thread action assertions to match REST-backed behavior.

12. `tests/e2e/specs/permissions-settings.spec.ts` and `tests/e2e/specs/skills-settings.spec.ts`

- Update settings CRUD assertions to match REST-backed behavior.

13. `tests/e2e/specs/helpers.ts` (if needed)

- Adjust helper waits/assertions that currently assume WS CRUD event contracts.

## Testing and Validation

1. Backend tests:

- `cargo test`
- Add REST route tests for thread CRUD/history, permissions read/write, and skills read/write.

2. Frontend checks:

- `cd web && bun run build`
- Verify app-store CRUD/settings actions call REST via `axiosInstance`.

3. E2E migration validation:

- `mise run test:e2e`
- Ensure CRUD/settings-oriented specs pass after migration:
  - `thread-lifecycle.spec.ts`
  - `slash-commands.spec.ts`
  - `threads-explorer.spec.ts`
  - `permissions-settings.spec.ts`
  - `skills-settings.spec.ts`
  - `settings-route.spec.ts`

4. WebSocket runtime regression validation:

- Confirm WebSocket-focused specs still pass:
  - `chat-agent-response.spec.ts`
  - `kill-switch.spec.ts`
  - `tool-approval.spec.ts`
  - `mcp-settings.spec.ts`

5. Rollback check:

- Revert migration patchset in a test branch and rerun baseline backend + E2E commands to confirm reversibility.

## Acceptance Criteria

1. Migrated CRUD/settings operations are available as REST endpoints listed in this PRD.
2. Frontend uses `axiosInstance` (with configured `baseURL`) for migrated CRUD/settings operations.
3. Migrated WS CRUD/settings handlers are removed from backend and no legacy compatibility path remains.
4. `kill_switch` and `mcp_status` remain WebSocket-based.
5. Session selection is handled client-locally and API contracts no longer require server-global current session behavior.
6. Backend tests for REST endpoints pass.
7. Updated Playwright E2E suite passes for both migrated CRUD/settings flows and retained WebSocket runtime flows.

## Risks and Mitigations

- **Risk**: Hard cutover can break clients if frontend/backend versions are deployed out of sync.
  - **Impact**: CRUD/settings features fail immediately in mismatched deployments.
  - **Mitigation**: Ship frontend/backend together and gate release with full backend + E2E test pass.

- **Risk**: Session-local selection changes can cause stale or incorrect UI state.
  - **Impact**: Wrong thread appears active or history load inconsistencies.
  - **Mitigation**: Make thread selection explicit in app-store state transitions and add E2E coverage for switch/create/delete flows.

- **Risk**: E2E suite brittleness during transport migration.
  - **Impact**: False negatives slow down rollout.
  - **Mitigation**: Update shared helpers and keep assertions focused on visible behavior rather than transport internals when possible.

- **Risk**: WS handler removal may accidentally remove required runtime events.
  - **Impact**: Chat streaming/tool approval regressions.
  - **Mitigation**: Keep explicit WS runtime contract list and run WebSocket-specific E2E specs in the migration gate.

## Open Questions

- None currently.
- Decision: `kill_switch` remains WebSocket-only.
- Decision: `mcp_status` remains WebSocket push-only.
- Decision: session selection is client-local; server focuses on session list and per-session data APIs.
- Decision: migrated WS CRUD/settings handlers are removed in this change (no legacy compatibility mode).

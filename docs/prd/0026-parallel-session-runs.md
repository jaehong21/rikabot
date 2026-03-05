# Parallel Session Runs, Queued Input, and Incremental Transcript Persistence

## Context

- Current runtime orchestration is globally single-run:
  - `src/gateway/mod.rs` stores one `RunManager.active: Option<ActiveRunState>`.
  - `src/gateway/ws.rs` rejects new `message` events while any run is active (`"A run is already active..."`), regardless of session.
- Current web runtime state is also single-session-at-a-time:
  - `web/src/context/app-store.tsx` has global `entries`, `isWaiting`, and `killRequested` (not session-scoped).
  - `sendMessage` returns early when `stateRef.current.isWaiting` is true, so normal messages and slash commands are effectively blocked while running.
  - `switchThread` hard-blocks when `snapshot.isWaiting`.
  - UI controls are disabled while waiting in:
    - `web/src/routes/chat-page.tsx` (`Textarea disabled={state.isWaiting}`).
    - `web/src/components/left-rail.tsx` (`disabled={state.isWaiting}` for session buttons).
    - `web/src/components/command-palette.tsx` and `web/src/routes/threads-page.tsx`.
- Chat route state is not query-driven today:
  - `web/src/router.tsx` defines chat route without `validateSearch`.
  - There is no canonical `session` query string like `/settings?section=...`.
- Transcript persistence is late and success-only:
  - `src/gateway/ws.rs` appends messages to `{session_id}.jsonl` only after run completion and only when `outcome.result` is `Ok(())`.
  - If a run is aborted (`kill_switch`) or fails before successful completion, that turn can be missing from session storage.
- User-requested behavior:
  - run multiple sessions in parallel.
  - navigate sessions while runs are in progress.
  - keep slash commands usable while runs are in progress.
  - queue submitted user input during active session runs, auto-dispatch queued inputs after completion, and allow queue cancellation.
  - make chat route session selection query-driven (similar to settings section query).
  - append transcript entries incrementally so refresh/reconnect after failure still shows history.
- Reference direction:
  - `nanobot` session manager patterns for append-oriented persistence.
  - `zeroclaw` Rust session isolation and run-loop behavior.

## Goals

1. Enable concurrent agent runs across different sessions (session-scoped concurrency, not global singleton).
2. Allow users to switch between sessions and routes while runs continue in the background.
3. Keep chat composer enabled during active runs and support queued user inputs per session.
4. Keep slash commands available while a run is active.
5. Make chat session selection URL-query-driven using a canonical session query parameter.
6. Persist transcript incrementally during turns so refresh/reconnect remains reliable even on failure or kill switch.
7. Preserve existing thread CRUD, settings, and tool-approval behavior.

## Non-Goals

1. Multi-user authorization, workspace tenancy, or per-user run isolation.
2. Replacing WebSocket runtime streaming with REST.
3. Redesigning `ChatMessage` storage format away from JSONL role/content lines.
4. Priority scheduling, weighted queue fairness, or queue reordering beyond FIFO per session.
5. Offline draft autosave of unsent composer text.
6. Database migration for session/run state persistence.

## Requirements

### Functional Requirements

1. Session-scoped active runs and cross-session concurrency:

- The backend must allow one active run per `session_id`.
- Multiple sessions may run concurrently up to a global cap `max_concurrent_sessions`.
- `max_concurrent_sessions` defaults to `8`.
- `max_concurrent_sessions` must be configurable in `config.toml` (e.g. `max_concurrent_sessions = 8`).
- Submitting a message to session B must not be blocked by session A’s active run when the global cap is not reached.

2. Per-session input queue:

- When a message is submitted to a session with an active run, the backend must enqueue it (FIFO) instead of rejecting it.
- When a message is submitted while global active sessions already reached `max_concurrent_sessions`, the message must be enqueued until a run slot is available.
- Queue storage is in-memory only (not persisted across backend restart in this phase).
- Queue length is capped at 5 queued user messages per session.
- On completion (`done`) of the active run, the next queued input for that session must auto-start immediately.
- `done` from any active session must also trigger scheduler evaluation for queued sessions waiting on global cap availability.
- On runtime `error`, queued inputs must remain queued (no auto-run).
- On user stop (`kill_switch` / `stopped`), all queued items for that session must be removed.
- Queue items must be cancellable (single item and clear-all for a session).
- Queue state changes must be pushed to connected clients.

3. Runtime event scoping:

- Every run lifecycle event sent to web clients must include `session_id` and `run_id`.
- This includes `user_message`, `chunk`, `tool_call_start`, `tool_call_result`, `tool_approval_required`, `done`, `stopped`, and runtime `error`.
- Event payloads must remain backward-compatible for existing fields.

4. Web session runtime model:

- Frontend state must be session-scoped (entries, waiting status, kill status, queue).
- Switching session must not reset/lose background session state.
- UI must render the selected session transcript while preserving background run progress in other sessions.

5. Navigation while running:

- Session switches from left rail, thread explorer, and command palette must remain enabled during active runs.
- `switchThread` logic must no longer hard-block on global waiting.

6. Composer and slash command behavior:

- Composer input area must remain enabled during active runs.
- Submitting normal text while selected session is running enqueues the message.
- Slash commands must remain executable while a run is active.
- `/stop` must continue targeting the selected session’s active run and must clear that session’s queue.
- Existing slash commands (`/new`, `/rename`, `/clear`, `/delete`, `/tools`, `/help`) must remain available while runs are active.

7. Queue cancellation UX:

- Chat page must surface queued inputs for the selected session with cancel actions.
- Cancel action must remove item(s) from backend queue and update UI immediately.
- Queue UI must distinguish pending items from active run state.

8. Query-backed chat session selection (`$web-spec` alignment):

- Canonical chat URL form: `/?session=<session_id>`.
- Chat route search must be typed and normalized in `web/src/router.tsx` using `validateSearch`.
- Missing or invalid `session` query must resolve to the first available session.
- URL query must be source of truth for selected chat session:
  - refresh preserves selected session.
  - browser back/forward restores selected session.
  - navigation actions update query via TanStack Router `navigate({ to, search })`.

9. Incremental transcript persistence:

- `{session_id}.jsonl` must be appended as turn artifacts are produced, not only after successful completion.
- At minimum, append immediately when each of the following is created:
  - accepted user message
  - assistant tool-call message(s)
  - tool result message(s)
  - final assistant response message
  - stopped/error assistant note when no final response exists
- Persistence must occur for success, failure, and kill-switch paths.

10. Refresh/reconnect safety:

- On page refresh during active runs, the client must reconnect and continue receiving in-flight events for active sessions.
- Persisted history must already include artifacts written before refresh/failure.
- Queue state for active sessions must resynchronize on reconnect.

### Non-Functional Requirements

1. Reliability:

- No cross-session event bleed (events must hydrate only matching session state).
- Queue operations and run transitions must be idempotent and race-safe.

2. Durability:

- Incremental appends must survive process crash or user refresh at any point in a turn.
- Session index (`sessions.json`) and message counts must remain consistent with append flow.

3. Performance:

- Event fanout and state updates should remain responsive with at least 8 concurrent active sessions and up to 5 queued user messages per session.
- Per-event persistence overhead must not block event streaming (use bounded async writes or lightweight append path).

4. Compatibility:

- Existing session files (`sessions.json`, `{session_id}.jsonl`) remain readable without migration.
- Existing slash command syntax remains valid.

5. Operability:

- Run logs should include `session_id`, `run_id`, queue transitions, and stop reasons for debugging.

## Architecture and Design Impact

1. Backend run manager redesign:

- Replace global singleton active run with session-scoped run state in `RunManager`.
- Suggested structure:
  - `HashMap<String, SessionRunState>`
  - `SessionRunState { active: Option<ActiveRunState>, queue: VecDeque<QueuedInput> }`
- Enforce global concurrent active run cap from config (`max_concurrent_sessions`, default `8`).
- Keep `run_id` monotonic globally for traceability.

2. WebSocket protocol extensions:

- Inbound:
  - keep `message` and `kill_switch` contracts.
  - add queue control messages (e.g., `queue_cancel`, optional `queue_clear`).
- Outbound:
  - add `session_id` + `run_id` to runtime events.
  - add queue state events (e.g., `queue_updated` / snapshot on connect).
- Reconnect behavior:
  - reattach current socket to all active session runs and replay buffered events.

3. Incremental persistence pipeline:

- Move persistence from “success-only at run end” to event-driven append path in `src/gateway/ws.rs`.
- Ensure kill/failure paths still commit produced messages before teardown.
- Continue using `SessionManager::append_messages` as the storage primitive; add narrow helper(s) if needed for single-message appends.

4. Frontend store refactor:

- Replace global `entries/isWaiting/killRequested` with per-session runtime map.
- Derived active view is selected by route query `session`.
- Preserve thread metadata list in existing shape to minimize API churn.

5. Routing contract update:

- Add chat route search typing + normalization in `web/src/router.tsx`.
- Keep route definitions centralized and TanStack Router search APIs as required by web spec.
- Update navigation calls in left rail, command palette, and threads explorer to preserve/use `session` query.

6. Tradeoffs:

- Event-scoped persistence increases write frequency but provides failure-safe history and refresh resilience.
- Server-side queueing increases runtime complexity but avoids client-only queue loss and supports multi-tab consistency.

## Implementation Plan

### Phase 1: Setup

1. Define runtime contracts:

- session-scoped run state model in backend.
- queue item schema and event payloads.
- updated websocket payload typing (`session_id`, `run_id`).
- `max_concurrent_sessions` config contract (default `8`, customizable in `config.toml`).

2. Define chat search schema in router (`session` query normalization).
3. Add baseline tests that codify current failure mode (single active run rejection, unsaved turn on stop/failure) to protect regression intent during refactor.

### Phase 2: Core Work

1. Backend concurrency + queue:

- refactor `RunManager` to per-session active + queue states.
- allow concurrent `spawn_active_run` across different sessions.
- enforce global `max_concurrent_sessions` cap (default `8`, config-driven).
- implement enqueue/dequeue/cancel flows and queue event broadcasts.
- ensure queue auto-dispatch happens only on `done` (not on `error`).

2. Backend runtime event scoping:

- include `session_id` and `run_id` in all outgoing runtime events.
- update reconnect attach logic to replay active events for all sessions.

3. Backend incremental persistence:

- append messages as user/tool/assistant artifacts are produced.
- remove success-only append gating.

4. Frontend store/session model:

- move runtime fields to per-session state map.
- route incoming events by `session_id`.
- update send behavior: immediate send if idle, enqueue if running.

5. Frontend UI interactions:

- keep composer enabled during runs.
- remove run-based navigation disable flags.
- render queue list with cancel actions.
- keep slash commands available during running state.

6. Router/query updates:

- add chat `validateSearch` and URL normalization.
- update navigations to include/preserve `session` query.

### Phase 3: Validation

1. Backend validation:

- run/session concurrency tests and queue transition tests.
- verify kill/failure persistence in `{session_id}.jsonl`.

2. Frontend validation:

- confirm route-query session persistence on refresh/back-forward.
- confirm navigation and slash command availability during active runs.

3. E2E validation:

- add scenarios for parallel sessions, queue behavior, queue cancellation, and refresh mid-run.

4. Rollback check:

- in a temporary rollback branch, revert this change set and rerun baseline checks to confirm previous behavior and clean rollback path.

## File-by-File Changes

1. `src/gateway/mod.rs`

- replace `RunManager.active` model with session-scoped active/queue structures and global active-session cap handling.

2. `src/config.rs`

- add `max_concurrent_sessions` to `AppConfig` with default `8`.

3. `config.toml`

- add commented template override for `max_concurrent_sessions`.

4. `src/gateway/ws.rs`

- remove global active-run rejection logic.
- add per-session enqueue/dequeue/cancel flow.
- scope events with `session_id`/`run_id`.
- enforce config-based global active-session cap when scheduling starts.
- append session transcript incrementally during run lifecycle.
- update reconnect attach/replay logic.

5. `src/agent/mod.rs` (if needed for canonical persistence hooks)

- optionally add explicit event(s) for history append points to avoid reconstructing persisted messages from UI-oriented events.

6. `src/session/mod.rs`

- add helper(s) to optimize incremental append path (single-message append and count/index consistency).
- ensure message_count updates remain correct under frequent appends.

7. `src/cli.rs`

- pass `max_concurrent_sessions` from loaded config into gateway runtime state initialization.

8. `web/src/router.tsx`

- add chat route search typing and `validateSearch` for `session` query.

9. `web/src/types/app.ts`

- extend runtime event types with `session_id`, `run_id`, and queue events.
- add queue item types and per-session runtime typing.

10. `web/src/context/app-store.tsx`

- refactor to per-session runtime state map.
- route all runtime events by session.
- enable queue submit/cancel behavior.
- keep slash command execution available during active runs.

11. `web/src/routes/chat-page.tsx`

- remove composer disable while running.
- show queued input list and cancel controls.
- maintain send/stop behavior for selected session.

12. `web/src/components/left-rail.tsx`

- remove run-based button disable and navigate with `session` query.

13. `web/src/components/command-palette.tsx`

- remove run-based session switch disable and navigate with `session` query.

14. `web/src/routes/threads-page.tsx`

- allow open/switch during running; route to chat with `session` query.

15. `tests/e2e/specs/*`

- add/update specs for:
  - concurrent runs in different sessions.
  - global active-session cap behavior (`max_concurrent_sessions`, default `8`, configurable).
  - queued message auto-dispatch after done.
  - queued message does not auto-dispatch after runtime error.
  - queue cancellation.
  - slash command usability while a run is active.
  - refresh safety with in-flight run and persisted partial history.

16. `src/gateway/ws.rs` and/or dedicated backend test modules

- add Rust tests for per-session run isolation, queue semantics, and persistence-on-failure/stop.

## Testing and Validation

1. Backend unit/integration:

- `cargo test`
- new tests for:
  - two concurrent active runs in different sessions.
  - global active-session cap default is `8` and override from config is respected.
  - same-session second prompt goes to queue.
  - queue cancel removes targeted item and does not execute it.
  - queued prompt auto-dispatches after `done` only.
  - queued prompt does not auto-dispatch on runtime `error`.
  - kill/failure still persists already-produced messages.

2. Frontend compile checks:

- `cd web && bun run typecheck`
- `cd web && bun run build`

3. E2E:

- `mise run test:e2e`
- include assertions for:
  - switching sessions while one run is active.
  - slash command execution while active.
  - queued prompt appears, can be canceled, and auto-runs after `done` only.
  - queued prompt remains queued after runtime `error`.
  - queued prompts are cleared when `/stop` is issued.
  - `/?session=<id>` persists across refresh/back-forward.
  - refresh mid-run still shows persisted history for user/tool/assistant artifacts produced before refresh.

4. Manual durability check:

- run a prompt that triggers tool calls.
- kill or induce failure mid-turn.
- inspect `sessions/{session_id}.jsonl` and confirm incremental entries are present.

5. Rollback/compatibility check:

- revert patchset on temporary branch and rerun:
  - `cargo test`
  - `cd web && bun run build`
  - `mise run test:e2e`
- confirm rollback is clean and no data migration is required.

## Acceptance Criteria

1. While session A is actively running and global active sessions are below `max_concurrent_sessions`, a prompt submitted in session B starts immediately (no global single-run rejection).
2. While a session is running, a new prompt submitted in the same session is queued and visible as queued in UI.
3. Queued prompt can be canceled before execution; canceled item never runs.
4. When active response ends with `done`, the next queued prompt for that session starts automatically without user re-submit.
5. When user issues `/stop`, the active run stops and all queued prompts for that session are cleared.
6. On runtime `error`, queued prompts do not auto-run and remain queued until user action or later successful completion flow.
7. Queue length per session is capped at 5 queued user messages.
8. Maximum concurrent active sessions default to `8` and can be customized via `config.toml` (`max_concurrent_sessions`).
9. Session switching from left rail, command palette, and thread explorer works while runs are active.
10. Slash commands remain usable during active runs.
11. Chat route supports `/?session=<session_id>`; refresh and back/forward preserve selected session.
12. Transcript appends incrementally to `{session_id}.jsonl` during run lifecycle (not success-only).
13. After kill-switch or failed run, produced messages up to failure point remain present after refresh.
14. Existing thread CRUD, settings pages, and tool approval flows continue to pass existing automated tests.

## Risks and Mitigations

1. **Risk**: Race conditions between queue transitions and reconnect subscriptions.

- **Impact**: Duplicate execution or dropped queued item.
- **Mitigation**: Guard queue operations under run-manager mutex, use deterministic state transitions, and add stress tests for enqueue/dequeue/cancel ordering.

2. **Risk**: Event scoping regression causes cross-session transcript contamination in frontend.

- **Impact**: High; wrong messages displayed or wrong kill target.
- **Mitigation**: Make `session_id` mandatory in runtime events and enforce session-keyed reducers with strict type guards.

3. **Risk**: Frequent incremental appends increase IO and update churn.

- **Impact**: Throughput degradation under heavy tool usage.
- **Mitigation**: Keep append path minimal, batch closely-related writes where safe, and verify performance with concurrent session load tests.

4. **Risk**: Route/search normalization errors create invalid session URL loops.

- **Impact**: Navigation instability.
- **Mitigation**: Router-level `validateSearch`, explicit fallback-to-first-session logic, and E2E back/forward tests.

## Open Questions

None (resolved in this revision):

1. Queued prompts are in-memory only for this phase.
2. `/stop` clears all queued prompts in the session.
3. Queue length cap is 5 queued user messages per session.
4. Max concurrent active sessions default to `8` and are configurable via `config.toml` (`max_concurrent_sessions`).

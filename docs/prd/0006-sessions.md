# Session Persistence and Thread Management (Backend + Web)

## Context

Current behavior stores chat history only in-memory per WebSocket connection (`src/gateway/ws.rs`), so context is lost on reconnect/reload and there is no thread management in web UI.

We need persistent, file-based sessions with distinct context/history per thread, plus web features to manage threads.

Reference implementations:

- `nanobot`: workspace-local JSONL session storage and session manager patterns  
  (`$HOME/Desktop/open-source/nanobot/nanobot/session/manager.py`)
- `zeroclaw`: Rust conversation/session isolation semantics and clear/new behavior  
  (`$HOME/Desktop/open-source/zeroclaw/src/agent/loop_.rs`, `$HOME/Desktop/open-source/zeroclaw/src/memory/sqlite.rs`)

## Goals

1. Add persistent session storage under `{workspace_dir}/sessions`.
2. Ensure each thread/session has distinct context and chat history.
3. Use:
   - `sessions.json` for session list/metadata
   - `{session_id}.jsonl` for per-session message history
4. Implement web thread features:
   - create new thread
   - rename thread display name
   - clear context in current thread
   - delete thread
   - move around thread (switch active thread)
5. Keep existing tool-call loop behavior unchanged.
6. Store a thread `display_name` for web UI:
   - default `display_name` = `session_id`
   - allow user-driven rename in this phase

## Non-Goals

- Cross-user/multi-tenant session ownership and auth.
- Database-backed storage.
- Thread search/pinning in this phase.
- Migrating external channel sessions (Slack/Discord/etc.).

## Functional Requirements

### Session storage location

- Resolve workspace directory from `AppConfig.workspace_dir` (existing behavior in `src/main.rs`).
- Session root path: `{workspace_dir}/sessions`.
- Create directory on startup if missing.
- `session_id` must be UUID-based.

### File format

#### 1) `sessions.json`

- Single source of truth for thread metadata/order.
- Stored at `{workspace_dir}/sessions/sessions.json`.
- Proposed schema:

```json
{
  "version": 1,
  "current_session_id": "550e8400-e29b-41d4-a716-446655440000",
  "sessions": [
    {
      "id": "550e8400-e29b-41d4-a716-446655440000",
      "display_name": "550e8400-e29b-41d4-a716-446655440000",
      "created_at": "2026-02-27T12:34:56Z",
      "updated_at": "2026-02-27T12:35:20Z",
      "message_count": 12
    }
  ]
}
```

#### 2) `{session_id}.jsonl`

- Stored at `{workspace_dir}/sessions/{session_id}.jsonl`.
- Append-only message log (one JSON object per line), example:

```jsonl
{"role":"user","content":"Hello","timestamp":"2026-02-27T12:34:56Z"}
{"role":"assistant","content":"Hi!","timestamp":"2026-02-27T12:34:58Z"}
{"role":"assistant","content":"{\"tool_calls\":[...],\"content\":\"\"}","timestamp":"2026-02-27T12:35:01Z"}
{"role":"tool","content":"{\"tool_call_id\":\"call_1\",\"content\":\"...\"}","timestamp":"2026-02-27T12:35:02Z"}
```

Notes:
- Keep compatibility with current `ChatMessage { role, content }` semantics.
- `timestamp` is additional metadata; runtime history reconstruction only requires `role` + `content`.
- `display_name` is UI-facing only and does not affect file naming.

### Session/thread operations

1. `create`:
   - Generate UUID-based `session_id`.
   - Create empty `{session_id}.jsonl`.
   - Insert metadata in `sessions.json`.
   - Set default `display_name = session_id`.
   - Set as current session.

2. `rename`:
   - Update only `display_name` in `sessions.json`.
   - Keep `session_id` and `{session_id}.jsonl` path unchanged.
   - Validate non-empty display name.

3. `switch` (move around thread):
   - Set active `session_id`.
   - Load selected session history from `{session_id}.jsonl`.
   - Future user messages append to this session only.

4. `clear current`:
   - Keep session metadata and `session_id`.
   - Keep `display_name` unchanged.
   - Truncate current `{session_id}.jsonl` to empty.
   - Reset in-memory history for current connection/session.
   - Set `message_count = 0`, update `updated_at`.

5. `delete`:
   - Remove session entry from `sessions.json`.
   - Delete `{session_id}.jsonl` if exists.
   - If deleted session was current, switch to most recently updated remaining session; if none exists, auto-create a new empty session.

6. `list`:
   - Return all sessions sorted by `updated_at` (desc) for UI rendering.

All thread operations above are server-authoritative. The web client must call rikabot server APIs/WS controls and only reflect server-confirmed state.

## Architecture Changes

### Backend modules

- Add `src/session/mod.rs` with:
  - `SessionRecord` (metadata for `sessions.json`)
  - `SessionIndex` (`version`, `current_session_id`, `sessions`)
  - `SessionManager` (load/save/list/create/switch/clear/delete/append/load_history)
- Add `mod session;` in `src/main.rs`.

### App state wiring

- Extend `gateway::AppState` to include session manager handle.
- Construct manager from resolved workspace path in `main`.
- Use synchronization (`Arc<tokio::sync::Mutex<SessionManager>>`) to serialize file mutations safely.

### WebSocket protocol changes

Current client message type only supports `{type:"message"}`.
Add control messages:

- `{ "type": "thread_list" }`
- `{ "type": "thread_create", "display_name": "optional" }`
- `{ "type": "thread_rename", "session_id": "...", "display_name": "..." }`
- `{ "type": "thread_switch", "session_id": "..." }`
- `{ "type": "thread_clear" }`
- `{ "type": "thread_delete", "session_id": "..." }`

Add server events:

- `thread_list` with session metadata + current session id
- `thread_created`
- `thread_renamed`
- `thread_switched` (includes loaded history for hydration)
- `thread_cleared`
- `thread_deleted`
- existing `chunk/tool_call_start/tool_call_result/done/error` unchanged

### Web frontend changes (`web/src/App.svelte`)

- Add a thread sidebar/panel listing sessions.
- Add actions:
  - `New thread`
  - `Rename` (edit `display_name`)
  - `Clear` (current)
  - `Delete` (selected/current, simple confirmation)
  - thread selection (switch/move around)
- On load:
  - request thread list
  - switch to current session
  - hydrate message entries from loaded history
- Keep existing streaming/tool rendering behavior within active thread.

## Data Integrity and Concurrency

- Write `sessions.json` atomically: write temp file then rename.
- For `{session_id}.jsonl`, use append mode for message writes.
- Validate `session_id` to safe filename subset (`[A-Za-z0-9_-]`), reject invalid IDs.
- On malformed/missing files:
  - recover by recreating empty index or empty session file
  - emit warning logs
  - return user-visible error event only when operation cannot continue

## Test Requirements

- Implementation must include test code for non-trivial behavior and regression-prone flows.
- Do not add tests for trivial/obvious behavior (simple getters/setters, serde derive defaults, or one-line pass-through code).
- Prioritize tests for:
  - session file persistence and reload consistency (`sessions.json` + `{session_id}.jsonl`)
  - websocket thread operations (`create`, `rename`, `switch`, `clear`, `delete`) and state transitions
  - concurrency-sensitive mutation paths and atomic index writes
  - fallback behavior when deleting current session and when files are missing/corrupted

## Migration and Compatibility

- Existing runtime has ephemeral per-connection `history`.
- On first rollout:
  - initialize `sessions/` and `sessions.json` if absent
  - auto-create one default thread if list is empty
- No legacy migration required for current rikabot (no existing file-based session data).

## Implementation Plan

### Phase 1: storage + manager

1. Add `src/session/mod.rs` and tests for:
   - create/list/switch/rename
   - clear/delete
   - persistence reload
   - malformed/corrupt storage recovery behavior
2. Wire workspace dir resolution into gateway state.

### Phase 2: gateway/ws protocol

1. Extend `ClientMessage` parsing to support thread commands.
2. Replace per-connection anonymous `history` with active `session_id + history` loaded from manager.
3. Persist new messages/tool results to active session after each completed run.
4. Add websocket-level tests for non-trivial thread command handling and event payload/state correctness.

### Phase 3: web UI

1. Add thread list UI and actions (including rename).
2. Add message handlers for new thread events.
3. Hydrate entries on thread switch and keep existing render pipeline.

## Acceptance Criteria

1. After server restart, previous threads and histories remain available.
2. Creating a new thread yields isolated context (no message bleed from other threads).
3. Rename updates only `display_name` while preserving `session_id` and existing history file.
4. Clearing current thread removes its effective history but keeps the thread entry and display name.
5. Deleting thread removes metadata and file, and UI auto-selects a valid fallback thread.
6. Switching thread updates visible messages and subsequent prompts target the selected thread.
7. Existing message streaming/tool result UX remains functional.
8. New implementation includes meaningful non-trivial tests; trivial/obvious tests are intentionally omitted.

## Risks and Mitigations

- Risk: file corruption on crashes while writing index.
  - Mitigation: atomic replace for `sessions.json`.
- Risk: concurrent writes from multiple websocket clients.
  - Mitigation: shared async mutex around session manager mutation paths.
- Risk: UI/backend event ordering bugs on fast switching.
  - Mitigation: include `session_id` in thread events and ignore stale events client-side.

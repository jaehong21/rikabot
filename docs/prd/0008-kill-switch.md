# Kill Switch for Active Agent Run (Web + Gateway)

## Context

The web composer currently has only a submit button. While a request is running, UI enters `isWaiting` and there is no way to stop the run.

Current gateway flow in `src/gateway/ws.rs` also blocks on a single in-flight run (`event_rx.recv()` loop + `agent_handle.await`) before reading the next client message. That means a stop command cannot be received mid-run with the current structure.

We need a kill switch feature that lets users immediately stop the active run from web UI.

Reference patterns reviewed:

- `nanobot`
  - `nanobot/agent/loop.py` (`/stop` command cancelling active tasks)
- `zeroclaw`
  - `src/channels/mod.rs` (in-flight request tracking + per-request `CancellationToken`)
- `openclaw`
  - architectural direction for robust runtime control and event-driven agent orchestration

## Goals

1. Add a kill switch button in web composer, positioned to the right of submit.
2. Enable kill switch only while an agent run is active.
3. On click, stop the active run immediately (best effort) and return UI to idle.
4. Keep thread/session behavior stable (no corruption, no cross-thread side effects).

## Non-Goals

- Rolling back external side effects from tools that already executed.
- Guaranteed termination of every OS child process spawned by tools.
- Multiple concurrent runs per single web connection.
- Redesigning the whole chat protocol beyond kill-switch needs.

## Functional Requirements

### 1) Web UI: Kill switch button

- Add a dedicated `Kill` / `Stop` button in `web/src/App.svelte` composer area.
- Position: immediately right of the current submit button.
- Enabled state:
  - enabled when `isWaiting === true` and socket is connected
  - disabled otherwise
- Submit behavior remains unchanged (still disabled while waiting).
- Accessibility:
  - `type="button"`
  - clear `aria-label` (for example: `Stop response`)
  - visible disabled style distinct from active style

### 2) Client behavior and protocol

- Add outbound control event from web:
  - `{ "type": "kill_switch" }`
- On kill click:
  - send `kill_switch` once
  - prevent spam clicks until server acknowledges (idempotent UX)
- Add optional slash alias:
  - `/stop` should send the same `kill_switch` command
- Add inbound server event:
  - `{ "type": "stopped", "reason": "user_cancelled" }`
- Client handling of `stopped`:
  - set waiting state to false
  - finalize current streaming bubble
  - keep already-rendered partial output visible
  - do not expect a `done` event for the cancelled turn

### 3) Gateway runtime changes (required for real-time stop)

Refactor websocket connection handling into a connection-level state machine that can process:

1. incoming client commands,
2. outgoing agent events,
3. run completion,

at the same time (for example, `tokio::select!` over ws input + active run channels).

Connection must track at most one active run:

- `ActiveRun`
  - run id
  - `JoinHandle`
  - event receiver (`AgentEvent` stream)
  - session id snapshot

When `kill_switch` is received and run is active:

1. abort the active run task immediately (`JoinHandle::abort`),
2. mark run as cancelled in connection state,
3. stop forwarding any stale events from that run,
4. emit `stopped` event to client.

When `kill_switch` is received and no run is active:

- return a safe idempotent response (`stopped` with `reason: "no_active_run"` or equivalent benign error).

### 4) Agent and history semantics

- Cancellation is best effort and should happen quickly from user perspective.
- Cancelled turn must not emit `done`.
- Partial assistant chunks/tool blocks already sent to client may remain visible in UI.
- Persisted session history should remain consistent:
  - no partially-written malformed records
  - keep existing append semantics unless explicitly changed during implementation

### 5) Thread command behavior while active run

To keep scope controlled for this feature:

- while a run is active, thread-mutating commands (`thread_switch`, `thread_clear`, `thread_delete`, `thread_create`) should be rejected with explicit error, or require stop first.
- `thread_list` remains allowed.

## API/Event Schema Changes

### Client -> Server

- add: `kill_switch`

### Server -> Client

- add: `stopped`
  - fields:
    - `reason`: `user_cancelled | no_active_run | internal_cancel`
    - `session_id` (optional)

## Observability

Add logs/metrics for:

- kill switch requests received
- kill switch accepted/rejected
- cancel latency (click/receive to `stopped` emit)
- cancelled run count per session

## Test Requirements

Add tests for non-trivial behavior:

1. Gateway can receive `kill_switch` during active run (regression for current blocking loop).
2. Active run cancellation emits `stopped` and never emits `done` for that run.
3. Repeated `kill_switch` calls are idempotent.
4. Thread-mutating commands while run active follow defined rule (error or gated).
5. Session data remains valid after cancellation (no corrupt JSONL/index behavior).
6. Web UI state:
   - stop button enabled only during `isWaiting`
   - click sends `kill_switch`
   - client exits waiting state on `stopped`

## Implementation Plan

### Phase 1: gateway cancellation control

1. Refactor websocket loop to concurrent command/event handling.
2. Introduce active-run tracking with abort support.
3. Add `kill_switch` command and `stopped` event.
4. Add gateway tests for stop behavior and event ordering.

### Phase 2: web kill switch UX

1. Add stop button right of submit.
2. Wire click + `/stop` to `kill_switch`.
3. Handle `stopped` event and restore idle composer state.
4. Ensure button state rules and accessibility.

### Phase 3: hardening

1. Add logs/metrics for cancellation lifecycle.
2. Verify manual behavior with long-running tool calls.
3. Update docs for new client/server event types.

## Acceptance Criteria

1. While a response is running, web shows enabled kill switch next to submit.
2. Clicking kill switch stops the in-flight run and UI leaves waiting state without requiring reconnect.
3. No `done` event is emitted after a successful user-triggered stop.
4. Existing thread/session data remains consistent after cancellation.

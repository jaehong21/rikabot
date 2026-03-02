# 020 — CLI System Event Trigger for Delegated Agent Completion

## Context

Current `coding-agent` skill behavior in `~/.rika/workspace/skills/coding-agent/SKILL.md` is process-lifecycle only:

1. spawn delegated run
2. poll via `status` / `wait` / `output`
3. summarize when complete

This can hit iteration ceilings for long-running delegated tasks.

We need a push-based completion trigger similar to OpenClaw’s completion pattern, but implemented as **CLI-only** for this phase.

Primary requirement:

1. delegated worker runs `rika system event ...` when done
2. that event immediately triggers a new agent turn
3. if session is specified, continue that session
4. if session is not specified, create a new session

This will be the base for future heartbeat/cron work, but heartbeat/cron execution is out of scope in this PRD.

Reference behavior reviewed:

1. OpenClaw coding-agent completion trigger pattern.
2. Nanobot queue-based async handoff model.
3. Zeroclaw event/webhook hardening patterns (reviewed but deferred for this PRD).

## Goals

1. Add a CLI-only event trigger command for push-based completion signaling.
2. Ensure event-triggered runs can continue an existing session by `session_id`.
3. Persist event records under workspace before processing.
4. Clean up event records after processing is done.
5. Inject current session metadata (`session_id`, `session_display_name`) into agent context so coding-agent can reference them.

## Non-Goals

1. Gateway/API event ingestion.
2. Webhook aliases.
3. Secret/auth/signature checks.
4. Idempotency key handling.
5. Structured event payloads beyond text.
6. Heartbeat scheduler/cron engine.

## Scope

### In scope

1. New CLI command: `rika system event`.
2. Event file persistence lifecycle in `workspace/events/`.
3. Session-targeted event run behavior.
4. Session-context injection for ongoing agent turns.
5. Coding-agent skill guidance update to use session-aware completion trigger.

### Out of scope

1. HTTP endpoints for events.
2. Remote caller support.
3. Replay/dedup protocols.
4. Full scheduler execution logic.

## Functional Requirements

### 1) CLI command contract

Add command namespace:

1. `rika system event --text "<message>" [--session-id <uuid>] [--session-display-name "<name>"] [--json]`

Flags:

1. `--text <text>`: required; non-empty trimmed text.
2. `--session-id <uuid>`: optional; when set, event must run in that existing session.
3. `--session-display-name <name>`: optional; used only when creating a new session (ignored if `--session-id` is provided).
4. `--json`: optional machine-readable output.

Behavior:

1. If `--session-id` is provided:
   - validate UUID format
   - ensure session exists
   - run event in that session
2. If `--session-id` is omitted:
   - create a new session
   - use `--session-display-name` when provided, otherwise use default naming
   - run event in that new session
3. Command executes the triggered run immediately and returns final status.

### 2) Event persistence and cleanup

Before run starts:

1. persist event record to `workspace/events/<event-id>.json`

Suggested record fields:

1. `event_id`
2. `text`
3. `session_id`
4. `session_display_name`
5. `created_at`
6. `status` (`pending` | `running` | `done` | `failed`)

Lifecycle:

1. write event file in `pending`
2. mark/update as `running`
3. finish run (`done` or `failed`)
4. remove event file on completion cleanup

Cleanup policy for this PRD:

1. event files are transient and removed after run process completes

### 3) Event-triggered run execution

Use existing agent runtime stack (same model/tools/session persistence path as normal runs), but initiated from CLI command path.

Behavior:

1. append event text into target session as user message input for a normal turn
2. execute agent loop
3. persist appended assistant/tool messages to that session history
4. return concise command result (success/failure + session id)

### 4) Session context injection

Current gap:

1. agent does not reliably see its own current `session_id` and `session_display_name`

Requirement:

1. inject current session metadata into prompt context for each run

Injected fields (minimum):

1. `session_id`
2. `session_display_name`

This applies to:

1. WebSocket-driven runs
2. CLI-triggered `system event` runs

### 5) Coding-agent completion trigger contract

Update coding-agent guidance so delegated prompts include:

```text
When completely finished, run this command to notify me:
rika system event --text "Done: [brief summary of what was built]" --session-id [current session id]
```

Behavior:

1. use ongoing session id when known (preferred)
2. if session id is unavailable, allow `rika system event --text ...` and let command create a new session

## Config Model

No new auth/idempotency/system-event secret config in this PRD.

Only filesystem location requirement:

1. event files must be stored under resolved workspace path: `workspace/events/`

## Implementation Plan

### Phase 1: CLI surface

1. extend `src/cli.rs` with `system event` subcommand
2. parse `text`, `session_id`, `session_display_name`, `json`
3. wire command handler

### Phase 2: Event persistence + run path

1. add event record module for `workspace/events/` write/update/delete lifecycle
2. load/create target session based on flags
3. trigger one run through existing agent/session infrastructure
4. cleanup event file on completion

### Phase 3: Session context injection

1. extend prompt building path to include session metadata block
2. pass session metadata from both WebSocket and CLI command entrypoints

### Phase 4: Skill/doc updates + tests

1. update coding-agent skill instructions with `--session-id` completion trigger usage
2. add CLI parsing tests
3. add event file lifecycle tests
4. add session-routing tests (existing session vs auto-created)
5. add prompt injection tests for `session_id` and `session_display_name`

## File-by-File Changes (planned)

1. `src/cli.rs`
   - add `system event` command parsing and handling
2. `src/system_events/mod.rs` (new)
   - event record lifecycle (`pending/running/done/failed`, cleanup)
3. `src/session/mod.rs`
   - add helper(s) needed to fetch session record by id for display-name-aware routing
4. `src/prompt/mod.rs`
   - support session metadata injection into prompt output
5. `src/gateway/ws.rs`
   - pass session metadata when building prompt for websocket runs
6. `~/.rika/workspace/skills/coding-agent/SKILL.md`
   - update completion trigger section to include `--session-id`
7. `docs/prd/0020-event-trigger.md`
   - this design document

## Acceptance Criteria

1. `rika system event --text "Done: test" --session-id <existing-uuid>` appends a new turn to that existing session.
2. `rika system event --text "Done: test"` creates a new session and runs the event there.
3. Command persists event record under `workspace/events/` before execution.
4. Event record is cleaned up after event processing finishes.
5. Prompt context includes both `session_id` and `session_display_name` for active runs.
6. Coding-agent completion guidance includes session-aware trigger command.
7. No gateway endpoint is added for system events in this PRD.
8. No secret/auth/idempotency key logic is added in this PRD.

## Risks and Mitigations

1. Risk: concurrent session writes between websocket run and CLI event run.
   - Mitigation: reuse existing run/session locking patterns and serialize writes through session manager lock points.
2. Risk: event file orphaning on crash between persist and cleanup.
   - Mitigation: startup/command preflight can prune stale `running` event files from prior crashed executions.
3. Risk: delegated task sends event without session id and fragments history into new sessions.
   - Mitigation: inject session metadata into prompt and explicitly instruct coding-agent to pass `--session-id`.

## Decisions Applied

1. CLI-only implementation for this PRD (no gateway/webhook alias).
2. Persist event files first under `workspace/events/`, then clean up after event process completion.
3. Keep event payload text-only.
4. Defer signed/authenticated remote ingress.

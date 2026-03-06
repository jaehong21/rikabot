# Agent response streaming with refresh-safe hydration

## Context

- The web chat currently receives and renders `chunk` events, but agent responses are effectively non-streaming for normal (no-tool) completions because provider calls return full text at the end.
- During reconnect/refresh, websocket runtime replay events can race with initial REST history hydration. Late hydration can overwrite in-flight replayed state, causing perceived loss of streamed progress.
- User goal for this feature:
  - Stream assistant responses in the chat UI as they are produced.
  - Keep behavior robust across browser refresh while a run is still active.

## Goals

1. Deliver incremental assistant response updates to the chat UI while generation is in progress.
2. Preserve refresh safety for active runs: no lost in-flight assistant text after reconnect/hydration.
3. Maintain existing thread/tool-call behavior and final persisted transcript compatibility.
4. Keep implementation backward-compatible for providers that do not support native token streaming.

## Non-Goals

1. Persisting every token/chunk to session JSONL on disk.
2. Changing session file format (`sessions.json` and `{session_id}.jsonl`).
3. Introducing SSE endpoints to the browser (websocket transport remains unchanged).
4. Redesigning chat UI structure or visual system.

## Requirements

### Functional Requirements

1. Provider chunk streaming contract:

- Add a provider-level API that can emit incremental response text chunks during one chat completion call.
- Existing providers must remain compatible even if they only return final text.

2. OpenAI streaming support:

- `src/providers/openai.rs` must support OpenAI-compatible chat completion streaming (`stream: true`).
- Parse streamed `delta.content` updates and emit chunks immediately.
- Parse streamed tool-call deltas (`delta.tool_calls`) so tool-call loops continue to work.
- Preserve usage accounting when usage is included in streamed payloads.

3. Agent event emission:

- `src/agent/mod.rs` must forward provider chunks as `AgentEvent::Chunk` while the model call is still in flight.
- Avoid duplicate chunk emission for the same text.
- Keep existing `AgentEvent::Done` semantics with `full_response` and stats.

4. Refresh-safe frontend hydration:

- `web/src/context/app-store.tsx` must buffer inbound websocket runtime events until initial thread/history hydration is complete.
- Buffered events must be replayed in-order immediately after hydration.
- No event loss/overwrite should occur if replay arrives before REST hydration completes.

5. Chat page behavior:

- `web/src/routes/chat-page.tsx` must continue rendering streamed assistant text without requiring user interaction.
- Loading/wait indicators must remain consistent with in-progress runs.

6. Session JSONL persistence boundary:

- Keep current persistence cadence at turn artifacts (user message, tool call/result artifacts, final assistant response, stop/error note).
- Chunk-level incremental JSONL persistence is not required in this phase.

### Non-Functional Requirements

1. Reliability:

- No regression to thread switching, tool-call rendering, or queue handling.
- Reconnect path must be deterministic (ordered replay after hydration).

2. Compatibility:

- Non-streaming providers must still function via fallback behavior (single chunk or final-only behavior through provider fallback path).

3. Performance:

- Stream parsing must avoid blocking websocket fanout loop.
- No materially worse latency for final completion delivery.

4. Operability:

- Streaming failures should produce actionable errors and not leave the UI in a stuck waiting state.

## Architecture and Design Impact

1. Provider abstraction update:

- Extend `Provider` trait with an optional chunk-streaming method.
- Default implementation delegates to existing `chat` and emits full text as one chunk for compatibility.

2. OpenAI provider implementation:

- Add streamed response parsing from HTTP event stream.
- Aggregate final text/tool-calls/usage into `ChatResponse` while emitting chunk deltas in parallel.

3. Agent orchestration:

- Introduce an internal chunk-forward channel from provider calls to `AgentEvent::Chunk` emission.
- Keep existing iterative tool loop untouched except removing duplicate text chunk logic.

4. Frontend bootstrap ordering:

- Add “bootstrapping” gate in app store websocket handler.
- Queue `ServerEvent`s during initial REST fetch/hydrate, then flush in-order.

## Implementation Plan

### Phase 1: Setup

1. Extend provider contract for chunk callbacks in `src/providers/mod.rs`.
2. Add/adjust Rust test doubles implementing the updated trait (`src/gateway/ws.rs`, `src/gateway/rest.rs` tests).
3. Define frontend event buffering strategy in `web/src/context/app-store.tsx`.

### Phase 2: Core Work

1. Implement OpenAI streaming parser and chunk emission in `src/providers/openai.rs`.
2. Update agent loop in `src/agent/mod.rs` to consume provider chunks and emit `AgentEvent::Chunk` in real-time.
3. Add websocket bootstrap buffering + ordered flush in `web/src/context/app-store.tsx`.
4. Apply minimal chat route updates in `web/src/routes/chat-page.tsx` only if needed for streaming UX consistency.

### Phase 3: Validation

1. Backend validation:

- `cargo test` (full suite).
- Focus check on agent/gateway/provider behavior for done/chunk/tool-call paths.

2. Frontend validation:

- `cd web && npm run typecheck`.
- Manual check: send a long prompt, verify progressive text updates.

3. Refresh validation (manual acceptance path):

- Start a long-running response.
- Refresh browser mid-response.
- Verify transcript resumes with in-flight content and final message matches server output.

4. Rollback/compatibility check:

- Disable streaming path (temporary revert or fallback path) and verify standard `done` flow still completes and persists correctly.

## File-by-File Changes

1. `src/providers/mod.rs`

- Add `chat_with_chunks` method to `Provider` with default compatibility behavior.

2. `src/providers/openai.rs`

- Add streaming request fields and SSE/event-stream parser.
- Emit chunk deltas via callback channel.
- Aggregate streamed tool calls and usage into final `ChatResponse`.

3. `src/agent/mod.rs`

- Replace direct `provider.chat(...)` call with chunk-capable flow.
- Forward streamed chunks as `AgentEvent::Chunk` during generation.

4. `web/src/context/app-store.tsx`

- Add initial websocket event buffering during bootstrap hydration.
- Flush buffered events in-order after initial thread/history load.

5. `web/src/routes/chat-page.tsx`

- Confirm streamed message rendering path and waiting indicator behavior; minimal adjustments only if required.

6. `docs/prd/0027-agent-response-streaming-with-refresh-safe-hydration.md`

- This PRD document.

## Testing and Validation

1. Automated:

- `cargo test`
- `cd web && npm run typecheck`

2. Manual:

- Streamed response appears progressively in active chat session.
- Tool-call flows still render and resolve as before.
- Refresh mid-run does not drop in-flight message progress.
- Completed run persists full final assistant message in thread history.

## Acceptance Criteria

1. Assistant responses update incrementally in the UI before `done` for OpenAI-backed runs.
2. Refreshing the page during an active run preserves in-flight progress after reconnect and initial hydration.
3. Existing tool-call runs complete with unchanged behavior (tool start/result/approval flow remains intact).
4. Non-streaming-compatible providers continue producing valid final responses without crashes.
5. `cargo test` and `cd web && npm run typecheck` pass after implementation.

## Risks and Mitigations

- **Risk**: Streaming parser incompatibility across OpenAI-compatible backends.
  - **Impact**: Runtime errors or missing chunks in production.
  - **Mitigation**: Keep provider-level fallback behavior and add defensive parsing for optional/missing fields.

- **Risk**: Bootstrapping event queue introduces ordering bugs.
  - **Impact**: Duplicated or dropped entries after reconnect.
  - **Mitigation**: Strict in-order queue flush with deterministic bootstrap gate and targeted manual reconnect tests.

- **Risk**: Trait signature expansion breaks tests/mocks.
  - **Impact**: Build/test failures outside runtime code paths.
  - **Mitigation**: Update all test `Provider` impls in the same change and run full Rust test suite.

## Open Questions

- None for this phase.

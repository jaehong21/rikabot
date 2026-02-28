# Shared Workspace Memory and Prompt Files

## Context

`0006-sessions` defines thread/session-level isolation for chat history and runtime context.

This PRD adds **workspace-level shared memory and prompt files** so every session uses the same core assistant identity and long-term context, while keeping per-session chat history distinct.

Reference patterns reviewed:

- `nanobot`
  - `nanobot/templates/AGENTS.md`, `SOUL.md`, `USER.md`, `HEARTBEAT.md`
  - `nanobot/templates/memory/MEMORY.md`
  - `nanobot/agent/context.py` (bootstrap + memory prompt assembly)
- `zeroclaw`
  - `src/agent/prompt.rs` (Rust prompt section builder)
  - `src/channels/mod.rs` (workspace bootstrap injection semantics)
  - `src/memory/markdown.rs` (Markdown memory layout)
- `openclaw`
  - `docs/reference/templates/{AGENTS,SOUL,TOOLS,USER,IDENTITY,HEARTBEAT}.md`
  - `src/agents/workspace.ts` (bootstrap file loading/seeding)
  - `src/agents/bootstrap-files.ts`, `src/agents/system-prompt.ts` (prompt injection + limits)

## Goals

1. Add shared workspace prompt files for assistant identity/behavior.
2. Add shared workspace memory layout for durable notes.
3. Keep `0006` session histories isolated while sharing prompt/memory files across sessions.
4. Make prompt assembly deterministic, bounded, and safe for large files.
5. Keep implementation file-based (no DB dependency for prompt/memory files).

## Non-Goals

- HEARTBEAT task execution behavior changes (file may exist, logic is out of scope).
- Vector/semantic memory search backend.
- Per-session private memory files.
- Cross-user or multi-tenant memory isolation.

## Functional Requirements

### 1) Workspace file layout

Under `AppConfig.workspace_dir` (already used by sessions):

- `AGENTS.md`
- `SOUL.md`
- `TOOLS.md`
- `USER.md`
- `IDENTITY.md` (optional in behavior, but file should be scaffolded)
- `HEARTBEAT.md` (optional in behavior; scaffolded, not actively used by this PRD)
- `MEMORY.md` (optional curated long-term memory)
- `memory/` directory for daily notes
  - `memory/YYYY-MM-DD.md`

Notes:

- Session files from `0006` remain in `sessions/` and stay thread-specific.
- Prompt/memory files above are **shared across all sessions**.

### 2) File seeding behavior

On startup (or first prompt build), ensure workspace exists and seed missing bootstrap files with templates.

- Create if missing:
  - `AGENTS.md`, `SOUL.md`, `TOOLS.md`, `IDENTITY.md`, `USER.md`, `HEARTBEAT.md`
- Do not overwrite existing files.
- Do not auto-create `MEMORY.md` (optional; user/agent creates when needed).
- Ensure `memory/` directory exists.

### 3) System prompt assembly

Replace one-time static `system_prompt` behavior with per-turn assembly using shared workspace files.

Prompt structure:

1. Base prompt from `config.system_prompt`
2. Skills section (existing loader behavior)
3. Workspace bootstrap context (in fixed order):
   - `AGENTS.md`
   - `SOUL.md`
   - `TOOLS.md`
   - `IDENTITY.md`
   - `USER.md`
   - `HEARTBEAT.md`
   - `MEMORY.md` (only if present)

Injection rules:

- Include each injected file under its own header (e.g., `## /abs/path/AGENTS.md`).
- If file missing, inject short missing marker instead of failing request.
- Skip completely empty files.
- Apply truncation limits:
  - max chars per file (default: `20_000`)
  - max total chars across injected files (default: `150_000`)
- `memory/*.md` daily files are **not auto-injected**; they are accessed on-demand via existing file tools.

### 4) Shared memory semantics with `0006` sessions

- Session history remains distinct per session file (`sessions/{session_id}.jsonl`).
- Workspace prompt/memory files are global for the workspace.
- If one session updates `MEMORY.md` or `memory/*.md`, other sessions can use that change on next turn.
- Thread operations (`switch`, `clear`, `delete`) do not mutate workspace prompt/memory files.

### 5) Behavior guidance for memory writes

`AGENTS.md` template should instruct:

- Durable facts/preferences/decisions -> `MEMORY.md`
- Running notes/logs -> `memory/YYYY-MM-DD.md`
- "remember this" requests must be written to files (not implicit chat memory)

No new memory tool is required in this PRD; existing filesystem tools are used.

## Architecture Changes

### Backend

Add a workspace prompt module (suggested: `src/prompt/mod.rs`) with:

- `WorkspaceBootstrapFile` model
- `ensure_workspace_bootstrap_files(workspace_dir)`
- `load_workspace_bootstrap_files(workspace_dir)`
- `build_system_prompt(base_prompt, skills_section, files, limits)`

### Agent/Gateway wiring

Current `Agent` stores a static `system_prompt`. To support shared-file updates across sessions:

- Change agent execution API to accept a per-run prompt string, or
- Add a prompt manager in `AppState` that produces prompt per incoming user turn.

Recommended path:

- Keep `Agent` stateless w.r.t. prompt text for each run.
- Gateway resolves prompt at message handling time, then calls agent with resolved prompt.

### Config additions

Add optional prompt limits to config (defaults above):

```toml
[prompt]
bootstrap_max_chars = 20000
bootstrap_total_max_chars = 150000
```

If omitted, defaults apply.

Also add commented default examples in `config.toml`:

```toml
# [prompt]
# bootstrap_max_chars = 20000 # default: 20000
# bootstrap_total_max_chars = 150000 # default: 150000
```

## Template Requirements (Initial Content)

Add template files in-repo (for workspace seeding), minimally aligned to referenced projects:

- `AGENTS.md`: operating rules + memory write policy
- `SOUL.md`: persona/tone/boundaries
- `TOOLS.md`: local environment notes
- `IDENTITY.md`: name/vibe/emoji/avatar metadata
- `USER.md`: user profile/preferences
- `HEARTBEAT.md`: empty task scaffold (no runtime behavior change in this PRD)

## Error Handling

- Missing/unreadable bootstrap file: inject marker and continue.
- Invalid UTF-8 or read error for a file: warn, inject marker, continue.
- Workspace seeding failures: return startup error only if workspace cannot be created; otherwise continue with best-effort markers.

## Test Requirements

Add tests for non-trivial behavior:

1. Seeding
   - creates missing bootstrap files
   - does not overwrite existing custom files
2. Prompt assembly
   - deterministic file order
   - missing-file marker behavior
   - per-file + total truncation behavior
3. Shared-memory/session semantics
   - two distinct sessions retain separate chat histories
   - updates to shared `MEMORY.md` become visible in both sessions on next turn
4. Robustness
   - invalid/missing files do not crash request path

## Migration and Compatibility

- Existing `0006` session storage remains unchanged.
- On first run after this feature:
  - missing workspace prompt files are scaffolded
  - existing custom files are preserved
- No migration of prior session JSONL data is required.

## Implementation Plan

### Phase 1: workspace bootstrap + prompt builder

1. Add prompt workspace module and limits.
2. Add bootstrap template assets and seeding logic.
3. Add unit tests for seeding and prompt assembly.

### Phase 2: runtime wiring

1. Refactor agent/gateway prompt flow to resolve prompt per turn.
2. Keep existing session/thread protocol unchanged.
3. Add integration tests around shared prompt/memory visibility across sessions.

### Phase 3: docs and defaults

1. Update `AGENTS.md` guidance in workspace templates.
2. Add README/docs note for shared memory vs per-session history.

## Decisions Applied

1. Standardize on `TOOLS.md` only (no filename alias support).
2. Auto-inject `MEMORY.md` for all sessions in current setup.
3. Memory layout for this phase is only `MEMORY.md` + `memory/YYYY-MM-DD.md`.

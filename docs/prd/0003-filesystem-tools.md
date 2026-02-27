# Filesystem Tools

## Context

Rikabot currently exposes only a `shell` tool. The skills prompt tells the model to load on-demand skills with `cat <path>` in `src/skills/mod.rs`.

This is brittle and overly broad:

- Skill loading depends on shell command composition instead of a structured tool.
- The model has no typed schema for file reading (path/line ranges).
- `shell` remains the only path for filesystem access.

Reference implementation in zeroclaw:

- `../../open-source/zeroclaw/src/tools/file_read.rs`
- `../../open-source/zeroclaw/src/tools/mod.rs`

## Goals

1. Add first-class filesystem tools in `src/tools/`.
2. Make skills loading guidance use a filesystem tool (not `cat`).
3. Keep changes minimal and compatible with current agent/provider flow.
4. Add tests for tool behavior and prompt text updates.

## Non-Goals

- Full security policy subsystem parity with zeroclaw (rate limits, autonomy levels, policy engine).
- Replacing the `shell` tool in this PR.
- Large agent-loop or provider protocol changes.

## Plan

### Phase 1 (required): `filesystem_read`

Add `src/tools/filesystem_read.rs` with:

- Tool name: `filesystem_read`
- Description: read text files with optional line slicing
- Schema:
  - `path` (required, string)
  - `offset` (optional, 1-based line start)
  - `limit` (optional, max lines)

Behavior:

- Read file via Rust fs API (`tokio::fs::read_to_string`).
- Return line-numbered output (same UX pattern as zeroclaw `file_read`).
- Handle non-UTF8 with lossy conversion fallback (`tokio::fs::read` + `String::from_utf8_lossy`).
- Return structured `ToolResult` errors on missing path/read failures.

Path handling (MVP):

- Allow absolute or relative paths.
- Canonicalize when possible before reading.
- Reject directories.

### Phase 1 (required): register and expose tool

Update `src/tools/mod.rs`:

- `pub mod filesystem_read;`
- Register `FilesystemReadTool` in `default_registry()`.

No provider schema changes are required because tool specs are already dynamic from `ToolRegistry`.

### Phase 1 (required): switch skills instruction from `cat`

Update `src/skills/mod.rs` prompt builder text:

- Replace:
  - `To use a skill, read its full instructions: \`cat <path>\``
- With:
  - `To use a skill, read its full instructions with the filesystem tool: \`filesystem_read\``

This aligns the model instruction with the new typed tool path.

### Phase 2 (follow-up): additional filesystem tools

After `filesystem_read` is stable, add:

1. `filesystem_write` (`path`, `content`) for deterministic file writes.
2. `filesystem_glob` (`pattern`) for file discovery.
3. `filesystem_search` (`pattern`, optional `path/include`) for content search.

These map to zeroclaw patterns (`file_write`, `glob_search`, `content_search`) but can be added incrementally.

## File-by-File Changes

- `src/tools/filesystem_read.rs` (new)
- `src/tools/mod.rs`
- `src/skills/mod.rs`
- Tests in `src/tools/filesystem_read.rs` and `src/skills/mod.rs`

## Verification

1. Unit tests:
   - `filesystem_read` name/schema/line slicing
   - missing path and invalid path errors
   - non-UTF8 fallback behavior
2. Existing skills tests:
   - assert prompt contains `filesystem_read` instruction
   - assert no `cat <path>` string remains
3. Build and test:
   - `cargo build`
   - `cargo test`

## Risks and Mitigations

- Risk: model still prefers `shell` for file reads.
  - Mitigation: explicit skills guidance and clear tool description.
- Risk: unrestricted absolute paths may expose too much.
  - Mitigation: Phase 2 can add workspace-scoped allowlist/config once requirements are finalized.

## Open Question

Should `filesystem_read` be restricted to `workspace_dir` only from day one, or remain unrestricted for MVP compatibility?

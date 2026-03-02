# 020 — Configurable Shell Timeout + Process Tool for Long-Running Jobs

## Context

Rikabot currently hardcodes the `shell` tool timeout to 30 seconds in `src/tools/mod.rs` when registering `ShellTool`.

User requirement for this phase:

1. Confirm whether shell timeout is currently fixed at 30 seconds.
2. Make shell timeout configurable in `config.toml`.
3. Add a first-class long-running process/jobs tool (distinct from subagent delegation).
4. Support the `coding-agent` skill use case without relying on shell-timeout workarounds.

Observed `coding-agent` draft behavior today:

- `/Users/jetty/.rika/workspace/skills/coding-agent/SKILL.md`
- `/Users/jetty/.rika/workspace/skills/coding-agent/scripts/coding_agent_session.sh`

The draft skill currently works around the 30-second shell limit by launching background `nohup` sessions and managing pid/log/state files manually.

Reference patterns reviewed:

- `zeroclaw`
  - `src/tools/process.rs` (spawn/list/output/kill process lifecycle)
  - `src/tools/bg_run.rs` (background job wrapper and status polling)
  - `src/tools/subagent_spawn.rs`, `subagent_manage.rs`, `subagent_list.rs` (separate async agent delegation lifecycle)
- `nanobot`
  - `nanobot/agent/tools/shell.py` + `nanobot/config/schema.py` (`ExecToolConfig.timeout`)
  - `nanobot/agent/subagent.py` + `agent/tools/spawn.py` (subagent orchestration, separate from shell)

## Goals

1. Make shell timeout configurable with safe defaults and validation.
2. Add a native `process` tool for long-running local commands with session/job IDs.
3. Keep `process` and `subagent` as separate concepts with clear boundaries.
4. Enable `coding-agent` workflows (start/status/logs/wait/result/stop/list) via first-class tool behavior.
5. Preserve existing permissions and security controls for command execution.

## Non-Goals

1. Replacing or redesigning subagent architecture.
2. Full terminal/PTY emulation in this phase.
3. Distributed job orchestration across multiple machines.
4. Guaranteed persistence/recovery of running processes across server restarts.

## Scope

### In scope

1. New `shell` config section with `timeout_secs`.
2. New `process` tool with lifecycle actions.
3. Process job store in-memory, with bounded buffers and cleanup.
4. Permission + command policy integration for `process` actions.
5. Config/template/docs updates.
6. Tests for config validation and process lifecycle behavior.

### Out of scope

1. New web UI dashboard for process jobs.
2. System service manager integration (`launchd`, `systemd`).
3. Subagent provider/tooling changes.

## Why Process Tool (Not Subagent)

`process` and `subagent` solve different problems and should both exist.

| Capability | `process` | `subagent` |
| --- | --- | --- |
| Unit of work | OS command/process (`codex exec`, test runner, build) | Delegate LLM agent task |
| Lifecycle | pid/job state, stdout/stderr, signals | session/task state, provider loop |
| Output | stream/log snapshots | final model output (plus progress metadata) |
| Best for | long-running CLI tools and coding agents | parallel reasoning/delegation |

Decision: add `process` as command execution runtime primitive; do not overload subagent for this use case.

## Functional Requirements

### 1) Shell timeout configuration

Add top-level config:

```toml
[shell]
enabled = true
timeout_secs = 30 # default
max_output_bytes = 10000 # optional, keeps existing behavior by default
```

Requirements:

1. `timeout_secs` default remains 30 seconds for backward compatibility.
2. `timeout_secs` must be `> 0` and capped to a safe max (proposed: 3600).
3. Optional alias `timeout_seconds` should be accepted for consistency.
4. `default_registry(...)` must construct `ShellTool` using config value instead of hardcoded `30`.
5. Error output must include actual configured timeout on expiry.

### 2) Process tool contract

Tool name: `process`

Parameters schema (MVP):

- `action` (required): `spawn | list | status | output | wait | kill`
- `command` (required for `spawn`)
- `id` (required for `status`, `output`, `wait`, `kill`)
- `max_wait_secs` (optional for `wait`, bounded by config)
- `lines` (optional for `output`, default bounded tail)
- `approved` (optional bool, mirrors shell supervised approval semantics)

Behavior:

1. `spawn`
   - validate command with existing security/permission path used by shell.
   - start process without foreground wait.
   - return `{ id, pid, status: "running", started_at, command }`.
2. `list`
   - return all tracked processes with compact status.
3. `status`
   - return one process status (`running | finished | killed | failed`) and exit metadata.
4. `output`
   - return bounded stdout/stderr snapshot and truncation metadata.
5. `wait`
   - wait up to `max_wait_secs` (or configured default), then return current status.
   - timeout behavior is non-fatal (`still_running: true`) to support polling loops.
6. `kill`
   - send graceful terminate first, then force kill after grace window.

### 3) Process runtime and safety

1. Track jobs in an in-memory store keyed by `id`.
2. Limit concurrent running processes (proposed default: 8).
3. Capture stdout/stderr asynchronously with bounded ring buffers (proposed default: 512KB per stream).
4. Keep completed process metadata for a retention window (proposed default: 10 minutes), then clean up.
5. Ensure child processes are terminated best-effort on application shutdown.
6. For MVP, process state does not survive server restart.

### 4) Permissions and policy integration

1. `process` must be governed by existing `permissions.tools.allow/deny` engine.
2. Support selector-based rules, e.g.:
   - `process(action:spawn,command:codex exec *)`
   - `process(action:list)`
   - `process(action:status,id:*)`
   - `process(action:kill,id:*)`
3. Deny rules continue to override allow rules.
4. If permissions are enabled and allow list is empty, `process` remains default-denied.

### 5) Config model for process tool

Add top-level config:

```toml
[process]
enabled = true
max_concurrent = 8
max_output_bytes = 524288
cleanup_retention_secs = 600
kill_grace_secs = 5
wait_default_secs = 20
wait_max_secs = 25
```

Validation:

1. numeric fields must be `> 0`.
2. apply hard caps for resource safety.
3. if `enabled = false`, tool is not registered.
4. `wait_default_secs` must be `<= wait_max_secs`.

### 6) Coding-agent skill enablement target

The final capability target for this PRD is to make `coding-agent` viable through the native process API.

Mapping from current script workflow:

1. `start` -> `process(action:"spawn", command:"codex exec ...")`
2. `status` -> `process(action:"status", id:...)`
3. `logs`/`result` -> `process(action:"output", id:..., lines:...)`
4. `wait` -> `process(action:"wait", id:..., max_wait_secs:...)`
5. `stop` -> `process(action:"kill", id:...)`
6. `list` -> `process(action:"list")`

This removes dependence on ad-hoc `nohup` + manual pid/log file bookkeeping in skill scripts.

## Implementation Plan

### Phase 1: Config and shell timeout wiring

1. Add `ShellConfig` and `ProcessConfig` to `src/config.rs`.
2. Add defaults + validation + aliases.
3. Update `config.toml` template comments.
4. Pass shell/process config into tool registry construction.

### Phase 2: Process tool core

1. Create `src/tools/process.rs`.
2. Implement job store + action handlers.
3. Implement bounded stdout/stderr capture.
4. Add shutdown cleanup behavior.

### Phase 3: Permissions/security integration

1. Ensure `spawn` command follows same security checks as shell.
2. Add/verify selector matching behavior for process args.
3. Add tests for allow/deny process rules.

### Phase 4: Skill alignment and docs

1. Document recommended `coding-agent` flow using `process` tool semantics.
2. Keep existing shell-script fallback temporarily during rollout.
3. Add operator notes for migration from script-managed sessions.

### Phase 5: Verification

1. Unit tests for config parse/default/validation.
2. Unit tests for each process action.
3. Integration-style test: long-running command survives beyond shell timeout and remains observable/killable.

## File-by-File Changes

1. `src/config.rs`
   - add `ShellConfig`, `ProcessConfig`, defaults, validation
2. `config.toml`
   - add commented `[shell]` and `[process]` sections
3. `src/tools/mod.rs`
   - register `process` tool conditionally
   - wire `ShellTool` timeout from config
4. `src/tools/process.rs` (new)
   - process lifecycle implementation
5. `src/cli.rs`
   - pass new config to `default_registry(...)`
6. `src/permissions/mod.rs` (if needed)
   - ensure process selector/raw matching support is adequate
7. `docs/prd/0020-process-tool.md` (new)
   - this design document

## Acceptance Criteria

1. Shell timeout is configurable in `config.toml` and defaults to 30 seconds.
2. A shell command exceeding configured timeout returns timeout error with configured value.
3. `process spawn` starts a long-running command and returns an ID immediately.
4. `process status` reports running/completed state and exit metadata.
5. `process output` returns bounded stdout/stderr snapshots.
6. `process wait` supports bounded polling (non-fatal timeout behavior).
7. `process kill` terminates running process (graceful then forced when needed).
8. Process tool obeys existing permissions allow/deny policy.
9. End-to-end: `coding-agent`-style workflow is achievable via `process` actions without `nohup` script dependency.

## Risks and Mitigations

1. Risk: orphaned background processes.
   - Mitigation: best-effort kill on shutdown + TTL cleanup.
2. Risk: unbounded memory due to log capture.
   - Mitigation: ring buffer byte limits and truncation metadata.
3. Risk: expanded command execution attack surface.
   - Mitigation: reuse shell security validation and permission gating.
4. Risk: confusion between process tool and subagent tool.
   - Mitigation: explicit documentation and separate tool contracts.
5. Risk: some CLI tools need PTY for full fidelity.
   - Mitigation: call out PTY support as follow-up, keep MVP non-PTY.

## Decisions From Review

1. Process state persistence across server restarts is out of scope for now.
2. MVP process execution inherits workspace context only (no per-process cwd/env overrides).
3. `wait` behavior is configurable via `process.wait_default_secs` and `process.wait_max_secs`.
4. Do not add a dedicated `coding_agent` tool; build skill workflows on top of generic `process`.
5. PTY-backed execution is a follow-up and should be added if Codex/Pi reliability requires it.

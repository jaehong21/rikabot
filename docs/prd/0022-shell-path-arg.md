# 022 — Shell Tool `path` Argument with `workspace_dir` Default

## Context

Current `shell` tool contract only accepts:

- `command` (string, required)

Execution behavior today:

1. Command runs via `sh -c <command>`.
2. Runtime sets `current_dir` to the configured workspace root when available.
3. Permissions for `shell` are effectively command-centric (`shell(command:...)`), so users often need patterns like `cd ... && ...` inside command text.

Requested behavior:

1. Add an explicit `path` argument for shell execution directory.
2. If `path` is omitted, default to `workspace_dir`.
3. Enable permission rules like `shell(command:git pull,path:/some/dir)` instead of allowing `cd *`.

## Goals

1. Add optional `path` to `shell` tool schema and runtime.
2. Make omitted `path` resolve to effective `workspace_dir`.
3. Support path-aware permission matching for `shell`.
4. Keep backward compatibility for existing command-only shell permission rules.
5. Remove the need to embed `cd ... &&` in command text for directory targeting.

## Non-Goals

1. Replacing shell command execution with a non-shell process API.
2. Introducing per-tool sandboxing beyond current permission engine.
3. Expanding environment-variable interpolation semantics for `path` (for example `${HOME}`).
4. Changing `process` tool contract in this PRD.

## Functional Requirements

### 1) Shell tool contract

Update `shell` parameters schema:

```json
{
  "type": "object",
  "properties": {
    "command": { "type": "string" },
    "path": {
      "type": "string",
      "description": "Directory where the command will run. If omitted, workspace_dir is used."
    }
  },
  "required": ["command"]
}
```

Requirements:

1. `command` remains required.
2. `path` is optional.
3. `path` accepts absolute or relative directory paths.
4. Empty/whitespace-only `path` is treated as omitted (fallback path).

### 2) Path resolution and defaulting

Define effective run directory as `effective_path`:

1. If `path` is omitted: `effective_path = workspace_dir`.
2. If `path` is relative: resolve against `workspace_dir`.
3. If `path` is absolute: use as-is.
4. Normalize `effective_path` with canonicalization (`realpath`-equivalent) before execution.
5. `effective_path` must exist and be a directory; otherwise return tool error.

Error behavior:

1. If `path` omitted but workspace dir is unavailable, fail with explicit error.
2. If path resolution/canonicalization fails, fail before command execution.

### 3) Permission model integration

Path-aware permissions must work even when caller omits `path`.

Requirements:

1. Permission evaluation input for `shell` must include resolved execution path.
2. Support selector rules:
   - `shell(command:git pull,path:/Users/jetty/Desktop/channeltalk/devops/k8s)`
   - `shell(command:git pull,path:/Users/jetty/Desktop/*)`
3. Keep deny-over-allow precedence unchanged.
4. Preserve existing raw command rules (for example `shell(git pull *)`) for compatibility.

Compatibility rule:

1. Existing command-only rules must continue to match as before.
2. New path selectors are additive, not breaking.

### 4) Suggested allow-rule generation

When shell call is denied and system suggests an allow rule:

1. Prefer structured selector output including command and effective path.
2. Example suggestion:
   - `shell(command:git pull,path:/Users/jetty/Desktop/channeltalk/devops/k8s)`

This reduces over-broad approvals compared to command-only suggestions.

### 5) Provider and protocol propagation

Tool schema returned to model providers must include optional `path`.

Requirements:

1. OpenAI/OpenRouter tool conversion should automatically reflect updated shell schema.
2. No protocol-breaking changes required for existing tool call payloads without `path`.

## Implementation Plan

### Phase 1: Shell runtime contract update

1. Extend `src/tools/shell.rs` schema with optional `path`.
2. Add path parsing + effective path resolution helper.
3. Execute command with `current_dir(effective_path)`.
4. Add unit tests for omitted, relative, absolute, and invalid paths.

### Phase 2: Permission enrichment for shell path

1. Extend permission-argument enrichment path to include shell `effective_path`.
2. Ensure permission checks happen against the same path used by execution.
3. Add tests for selector rules on `command` + `path`.

### Phase 3: Rule suggestion and docs

1. Update suggested allow-rule generation for shell denials.
2. Update config/docs examples to prefer `shell(command:...,path:...)`.
3. Keep command-only examples for backward compatibility notes.

## File-by-File Changes (planned)

1. `src/tools/shell.rs`
   - add `path` schema field
   - add effective path resolution/defaulting
   - improve errors around invalid/non-directory path
2. `src/tools/mod.rs`
   - enrich shell args for permission matching with effective path
   - ensure enrichment has access to workspace dir default
3. `src/permissions/mod.rs`
   - preserve current shell raw matching behavior
   - add/adjust tests for shell selectors using `path`
4. `src/agent/mod.rs`
   - update suggested allow-rule generation for denied shell calls
5. `config.toml`
   - update permission rule examples to include `path` selector usage
6. `docs/prd/0022-shell-path-arg.md`
   - this PRD

## Acceptance Criteria

1. `shell({"command":"git pull"})` runs in resolved `workspace_dir` by default.
2. `shell({"command":"git pull","path":"devops/k8s"})` runs in `{workspace_dir}/devops/k8s`.
3. `shell({"command":"git pull","path":"/abs/repo"})` runs in `/abs/repo`.
4. Invalid or non-directory `path` returns deterministic tool error before command starts.
5. Permission rule `shell(command:git pull,path:/abs/repo)` allows only that directory.
6. Existing command-only rule `shell(git pull *)` still functions.
7. Denied shell call suggestion includes path-aware selector rule.

## Risks and Mitigations

1. Risk: path traversal or symlink escape ambiguity.
   - Mitigation: canonicalize path before permission evaluation and execution.
2. Risk: command-only legacy rules become unexpectedly stricter.
   - Mitigation: preserve legacy raw shell matching behavior unchanged.
3. Risk: mismatch between permission path and execution path.
   - Mitigation: compute one `effective_path` and reuse it for both permission and runtime.

## Decisions Applied

1. `path` is optional and defaults to `workspace_dir`.
2. Relative `path` is resolved from `workspace_dir`.
3. Path-aware selector permissions are first-class for shell.
4. Backward compatibility for existing command-only shell rules is required.

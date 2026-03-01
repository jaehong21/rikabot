# 013 — Remove `system_prompt` from Config

## Summary

Remove `system_prompt` from TOML configuration so users cannot inject raw system prompt text via `config.toml`.

Prompt content is assembled from workspace markdown files (bootstrap templates under the workspace), plus optional skills section.

## Motivation

- Keep prompt authoring in markdown files, not inline TOML strings.
- Align runtime behavior with workspace-template based prompt composition.
- Reduce risk of ad-hoc prompt drift from local config edits.

## Scope

### In scope

- Remove `system_prompt` field from `AppConfig`.
- Remove `default_system_prompt()` from config code.
- Remove `system_prompt` option from root `config.toml` documentation template.
- Keep runtime variable naming (`system_prompt`) in the agent/gateway flow.

### Out of scope

- Rewriting historical PRD files.
- Changing agent loop semantics.
- Adding new prompt sources beyond existing workspace markdown + skills.

## Implementation Notes

- `PromptManager::build_prompt()` remains the source of the runtime system prompt.
- Gateway resolves prompt per run and passes it to `Agent::run(system_prompt, ...)`.

## Acceptance Criteria

1. `config.toml` does not expose `system_prompt` as an option.
2. `AppConfig` no longer parses/contains `system_prompt`.
3. The runtime still sends a system message on each provider call.
4. `cargo test` passes.

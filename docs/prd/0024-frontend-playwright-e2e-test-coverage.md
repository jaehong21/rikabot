# Frontend Playwright E2E Test Coverage

## Context

- Backend Rust modules already have test coverage, but frontend behavior currently has no automated E2E safety net.
- The frontend is strongly coupled to backend WebSocket contracts (`thread_*`, `permissions_*`, `skills_*`, `mcp_status`), so regressions can happen even when backend unit tests pass.
- User direction for this scope:
  - use Playwright for frontend E2E tests,
  - avoid mocked WebSocket backends,
  - run the real backend server in test setup (same runtime shape as `mise run dev:be`, but dedicated E2E tasks are allowed),
  - prioritize frontend-backend interaction scenarios.
- Reference style: [Pocket ID](https://github.com/pocket-id/pocket-id)’s Playwright setup (`tests` folder + dedicated config + command-driven environment bootstrapping).

## Goals

1. Add a maintainable Playwright E2E harness for this repository.
2. Run frontend E2E against the real Rust backend process and real frontend process.
3. Cover critical frontend-backend interactions that do not require external LLM providers.
4. Make E2E runnable from `mise` with a single command for local and CI usage.
5. Isolate E2E runtime state (workspace/config/session files) so tests do not mutate normal developer state.

## Non-Goals

1. Add frontend unit tests in this phase.
2. Full browser matrix coverage (initially Chromium-only).
3. Full LLM-response correctness testing against external providers (OpenAI/OpenRouter).
4. End-to-end validation for every MCP server type or external OAuth flow.
5. Visual regression/screenshot diff testing.

## Requirements

### Functional Requirements

1. Playwright setup:

- Add Playwright as a `mise` tool using `"npm:playwright" = "latest"` in `mise.toml`.
- Add an E2E test command via `mise` (for example `mise run test:fe:e2e`).
- Keep E2E files under a dedicated test area (`tests/e2e`) with a dedicated Playwright config.

2. Real server orchestration in test runtime:

- Playwright must start and wait for:
  - backend process using the same behavior as `mise run dev:be` (real Rust gateway/WebSocket),
  - frontend process using the same behavior as `mise run dev:fe`.
- Tests must connect to the actual `/ws` endpoint served by backend (no mock WebSocket server).

3. Frontend-backend interaction coverage:

- Add E2E tests for thread lifecycle over WebSocket-driven backend events:
  - initial thread bootstrap visibility,
  - create thread,
  - rename thread (slash command path),
  - clear/delete flow (slash command path).
- Add E2E tests for permissions roundtrip:
  - load current permissions state from backend,
  - save updated rules from frontend,
  - verify persisted state after reload.
- Add one route/search-state check where frontend behavior depends on backend-connected app shell lifecycle (e.g. `/settings?section=permissions` retains section on reload).
- Add at least one core chat-response E2E scenario using a deterministic mock LLM endpoint (OpenAI-compatible HTTP endpoint) while still exercising the real backend agent pipeline.

4. Test environment isolation:

- Use an E2E-specific config file passed via `RIKA_CONFIG`.
- E2E config must use an isolated workspace directory (e.g. `.tmp/e2e-workspace`) and isolated config output target.
- Tests must not write to developer default config (`~/.rika/config.toml`) or default workspace.

### Non-Functional Requirements

1. Determinism and reliability:

- Tests should avoid external network dependency for pass/fail (except package/tool installation).
- Tests should avoid requiring valid OpenAI/OpenRouter keys for baseline scenarios.
- Suite should be stable under retry (`retries = 1` in CI).

2. Runtime and developer UX:

- Baseline E2E suite target runtime: <= 2 minutes on typical local dev hardware.
- Clear startup failure messages when backend/frontend ports are unavailable.

3. Safety:

- E2E must operate only in isolated temp directories under repository root.
- Cleanup should remove transient test artifacts (`playwright-report`, test results, temp workspace) from git tracking.

4. Compatibility:

- Existing backend test commands (`cargo test`, `mise run test:be`) remain unchanged.
- Existing frontend dev flow (`mise run dev:fe`) remains unchanged for non-test use.

## Architecture and Design Impact

1. Test harness model:

- Playwright acts as orchestration layer for two real processes:
  - backend (`mise run dev:be`-equivalent command),
  - frontend (`mise run dev:fe`).
- Browser tests interact only through visible UI and network side effects.

2. Backend dependency strategy:

- To keep startup independent from external provider credentials while still validating core chat response flow, E2E runtime includes a local mock LLM HTTP endpoint (OpenAI-compatible `/chat/completions` behavior).
- Backend still runs as the real Rust server and performs normal agent execution, but provider calls are redirected to the local mock endpoint for deterministic assertions.

3. State isolation:

- `RIKA_CONFIG` points to an E2E-only TOML file.
- E2E workspace path stores session data and any saved permissions during tests.
- This protects normal local data and avoids flaky cross-run contamination.

4. Tradeoff:

- Real backend increases confidence vs mocked WebSocket flows, but startup is heavier and can introduce timing sensitivity.
- Mitigate with explicit `webServer` readiness checks, serialized workers for critical stateful scenarios, and isolated workspace reset per run.

## Implementation Plan

### Phase 1: Setup

1. Add `"npm:playwright" = "latest"` in `mise.toml`.
2. Add `mise` task(s) for frontend E2E execution.
3. Create Playwright config and directory layout in `tests/e2e`.
4. Add E2E-specific backend config TOML and temp workspace conventions.
5. Add local mock LLM endpoint process and its startup task for E2E.
6. Add ignore rules for Playwright artifacts.

### Phase 2: Core Work

1. Wire Playwright `webServer` to launch both backend and frontend processes.
2. Wire Playwright `webServer` to launch local mock LLM endpoint process.
3. Implement thread lifecycle E2E specs (bootstrap/create/rename/clear/delete).
4. Implement permissions roundtrip spec (load/save/reload persistence).
5. Implement settings query-state persistence spec.
6. Implement core chat-response spec backed by mock LLM endpoint.
7. Add reusable helpers for stable selectors and session/workspace cleanup.

### Phase 3: Validation

1. Run frontend E2E locally via `mise` command and stabilize flaky selectors/timings.
2. Run backend regression tests (`cargo test`) to confirm no backend breakage from test harness additions.
3. Execute compatibility check with normal dev commands (`mise run dev:be`, `mise run dev:fe`) to ensure unchanged behavior.
4. Verify rollback safety by temporarily disabling E2E tasks/config and confirming runtime app remains unaffected.

## File-by-File Changes

1. `mise.toml`

- add `"npm:playwright" = "latest"` under `[tools]`.
- add E2E task(s) (e.g. `test:fe:e2e`) to run Playwright from repo root.

2. `tests/e2e/playwright.config.mjs`

- define base URL, reporters, retries, project config, and multi-server startup for backend/frontend/mock-llm.

3. `tests/e2e/specs/thread-lifecycle.spec.ts`

- cover initial thread state + create/rename/clear/delete workflows.

4. `tests/e2e/specs/permissions-settings.spec.ts`

- cover permissions load/save/reload persistence.

5. `tests/e2e/specs/settings-route.spec.ts`

- cover settings section query-state behavior under real backend connection.

6. `tests/e2e/config/rika.e2e.template.toml`

- E2E-only app config: isolated workspace path + provider base URL pointing to local mock LLM endpoint.

7. `tests/e2e/support/mock-openai.*`

- deterministic local OpenAI-compatible endpoint used only during E2E.

8. `.gitignore` (repo root and/or `tests/e2e/.gitignore`)

- ignore Playwright output directories and temporary E2E state directories.

9. `README.md` and/or `web/README.md`

- add short “Frontend E2E” section with setup and run commands.

## Testing and Validation

1. Frontend E2E execution:

- `mise run test:fe:e2e`

2. Backend regression:

- `cargo test`

3. Manual smoke checks:

- `mise run dev:be` + `mise run dev:fe` still work without E2E env vars.
- Open app, navigate threads/settings, confirm baseline behavior unchanged.

4. Compatibility and rollback check:

- Verify E2E config/workspace does not affect normal `~/.rika` files.
- Disable E2E-only tasks/config and confirm runtime app still launches normally.

## Acceptance Criteria

1. A Playwright E2E suite exists and is runnable via a single `mise` command.
2. The suite starts real backend + real frontend processes and does not use a mocked WebSocket backend.
3. The suite starts a local deterministic mock LLM endpoint and verifies at least one successful chat response through the real backend agent flow.
4. At least one passing E2E test validates thread lifecycle interactions through frontend UI and backend WebSocket events.
5. At least one passing E2E test validates permissions save/load persistence through frontend settings UI and backend config write/read.
6. E2E runs use isolated config/workspace paths and do not modify default `~/.rika` state.
7. Existing backend test and dev commands remain operational after integration.

## Risks and Mitigations

- **Risk**: Flakiness from multi-process startup race conditions.
  - **Impact**: Intermittent test failures and low trust in suite.
  - **Mitigation**: Use explicit server readiness checks, deterministic timeouts, and CI retry policy.

- **Risk**: Test state pollution across runs (sessions/config persisted between runs).
  - **Impact**: Non-deterministic assertions and local environment contamination.
  - **Mitigation**: Use isolated E2E config/workspace paths and clean temp artifacts before/after runs.

- **Risk**: Backend startup requires provider config even for non-LLM tests.
  - **Impact**: E2E bootstrap failures on machines without provider keys.
  - **Mitigation**: Use dedicated E2E backend task + local mock LLM endpoint, and point E2E provider base URL to the mock service.

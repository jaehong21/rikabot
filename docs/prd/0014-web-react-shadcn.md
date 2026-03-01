# 014 — Web UI Rewrite: React + shadcn

## Context

The current `web/` frontend is a single large page with mixed concerns and limited route boundaries.

This rewrite moves the web UI to a production-style architecture with a uniform service shell based on the provided designer reference.

## Objectives

1. Move to React route/page separation with TanStack Router for typed multi-path navigation.
2. Build static artifacts via Rsbuild/Rspack configured for static output.
3. Use `shadcn/ui` components broadly across the interface.
4. Keep backend static serving compatibility with `web/dist`.

## Non-Goals

1. Backend protocol changes.

## Information Architecture

### Routes

1. `/`
   - Chat workspace with transcript, tool call cards, and composer.
2. `/settings`
   - Settings workspace with tabs:
     - `General`
     - `Permissions`
     - `Skills`
     - `MCP Servers`
3. Additional feature routes
   - Route stack must support adding more typed routes beyond the two core pages.

## Layout Blueprint

### Chat Workspace

```text
+-----------------------------------------------------------------------------------+
| APP FRAME                                                                         |
| +----------------------+--------------------------------------------------------+ |
| | LEFT RAIL            | TOP BAR                                                | |
| | [New] [Search]       +--------------------------------------------------------+ |
| | Today                | Thread title                               [Tools Btn] | |
| |  - Thread A (active) +--------------------------------------------------------+ |
| |  - Thread B          |                                                        | |
| | Yesterday            | Message transcript (scroll)                            | |
| |  - Thread C          |  - assistant text                                      | |
| |                      |  - user text                                           | |
| |                      |  - tool call cards (collapsible)                       | |
| | [Settings]           |                                                        | |
| +----------------------+--------------------------------------------------------+ |
|                        | Composer: [message input..................][Send][Stop]| |
|                        +--------------------------------------------------------+ |
+-----------------------------------------------------------------------------------+
```

### Settings Workspace

```text
+-----------------------------------------------------------------------------------+
| APP FRAME                                                                         |
| +----------------------+--------------------------------------------------------+ |
| | LEFT RAIL            | TOP BAR                                                | |
| | (same)               +--------------------------------------------------------+ |
| |                      | Settings                                               | |
| |                      | [General][Permissions][Skills][MCP Servers]            | |
| |                      |                                                        | |
| |                      | Section content cards:                                 | |
| |                      | - Selects / Switches / Inputs                          | |
| |                      | - Skills list                                          | |
| |                      | - MCP status rows                                      | |
| +----------------------+--------------------------------------------------------+ |
+-----------------------------------------------------------------------------------+
```

## Technical Plan

1. Replace current `web/src/` shell with route-first React architecture.
2. Introduce TanStack Router with shared app frame and typed navigation links.
3. Migrate build tooling from Vite/Svelte to Rsbuild React static build.
4. Implement reusable `shadcn/ui` primitives and consume them throughout pages.
5. Preserve existing websocket protocol payloads and thread/tool/permissions behavior.

## Acceptance Criteria

1. `web` builds static output to `web/dist`.
2. App has persistent left rail and top shell across route pages.
3. `/` renders chat workspace.
4. `/settings` renders tabbed settings workspace with General, Permissions, Skills, MCP Servers.
5. TanStack Router route stack includes `/`, `/settings`, and at least one additional feature route.
6. shadcn components are the default primitives across layout and controls.
7. No backend protocol changes are required.

# 015 — Markdown Rendering Quality Improvements (Web Chat)

## Context

The chat UI rendered assistant/user message text via a minimal regex-based markdown function.

Observed issues:

1. GitHub-style markdown tables were not parsed and appeared as raw pipe text.
2. General markdown content (headings, lists, blockquotes, links, separators) looked flat and awkward due to missing prose styles.
3. Forced `<br>` line-height override made normal paragraph flow less natural.

## Objectives

1. Render common markdown syntax reliably, including tables.
2. Improve readability for non-table text responses without changing backend protocol.
3. Keep styling aligned with the existing web palette/token system.
4. Preserve security by keeping raw HTML disabled in markdown rendering.

## Scope

### In scope

1. Replace regex markdown renderer with a proper parser in the web client.
2. Add scoped `.message-prose` typography styles for markdown elements.
3. Keep table styling improvements while balancing general text readability.
4. Validate with web typecheck and production build.

### Out of scope

1. Backend-side markdown transformation.
2. Streaming protocol/event schema changes.
3. Rich markdown extensions beyond current parser defaults.

## Implementation

### 1) Markdown parser upgrade

- File: `web/src/lib/markdown.ts`
- Replace custom regex conversions with `markdown-it`.
- Parser config:
  - `html: false`
  - `breaks: true`
  - `linkify: true`

Result:

- Native markdown table parsing is enabled.
- Existing markdown features (inline code, fenced code, emphasis, links, lists, headings) are handled by parser instead of ad-hoc regex.

### 2) Message prose styling expansion

- File: `web/src/styles.css`
- Added scoped styles under `.message-prose` for:
  - paragraph spacing (`p + p`)
  - heading hierarchy (`h1`–`h4`)
  - ordered/unordered lists and list item rhythm
  - blockquote visual treatment
  - links and horizontal rules
  - table layout, separators, cell spacing, overflow behavior

Result:

- Tables remain readable and structured.
- Non-table responses have clearer visual hierarchy and better spacing.

### 3) Remove `<br>` line-height override

- File: `web/src/routes/chat-page.tsx`
- Removed `[&_br]:leading-4` from message containers.

Result:

- Paragraph and list rendering feels less cramped/artificial.
- Line flow is more consistent across mixed markdown content.

### 4) Dependency updates

- File: `web/package.json`
- Added:
  - runtime: `markdown-it`
  - dev types: `@types/markdown-it`

## Verification

Run in `web/`:

1. `bun run typecheck`
2. `bun run build`

Both passed after changes.

## Acceptance Criteria

1. Markdown tables render as HTML tables (not raw pipe text).
2. Headings/lists/quotes/links/HR render with readable spacing and hierarchy.
3. Existing message rendering path remains `renderMarkdown(...)` in chat route.
4. Web typecheck and build pass.

## Follow-ups (Optional)

1. A/B compare `markdown-it` `breaks: true` vs `breaks: false` for long-form readability.
2. Add snapshot-style rendering tests for representative markdown fixtures.
3. Tune assistant message max width and text size scale for dense technical answers.

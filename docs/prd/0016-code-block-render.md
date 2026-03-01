# 016 — Code Block Rendering Improvements (Web Chat)

## Context

Markdown rendering was upgraded in the web chat, but fenced code blocks still looked visually weak and had no syntax highlighting.

Observed issues:

1. Fenced blocks used generic prose styling with low visual distinction.
2. Code content had no token-level highlighting, reducing readability for technical answers.
3. Inline code and fenced code shared overlapping styles, creating inconsistent appearance.

## Objectives

1. Improve fenced code readability with syntax highlighting.
2. Support explicit fenced language tags and automatic language detection fallback.
3. Keep styling aligned with the existing web token palette.
4. Preserve existing markdown security posture (`html: false`).

## Scope

### In scope

1. Integrate a code highlighting library into markdown rendering.
2. Add markdown-it highlight hook for language-aware rendering.
3. Refine `.message-prose` code styles for fenced vs inline code.
4. Apply light-mode code-block theme styling consistent with app surfaces.
5. Validate with web typecheck and production build.

### Out of scope

1. Copy button / language badge UI for code blocks.
2. Server-side syntax highlighting.
3. Theme switching system (dark mode toggle behavior).

## Implementation

### 1) Highlighting engine integration

- File: `web/package.json`
- Added runtime dependency:
  - `highlight.js`

Result:

- The web client can perform tokenization and syntax highlighting at render time.

### 2) Markdown highlight hook with auto-detect fallback

- File: `web/src/lib/markdown.ts`
- Added `highlight.js` integration through `MarkdownIt` `highlight(...)` option.
- Behavior:
  - If fenced language is provided and recognized, use explicit language highlighting.
  - Otherwise, use `highlightAuto(...)` fallback.
- Returned HTML structure:
  - `<pre class="hljs"><code>...</code></pre>`

Result:

- Code blocks now render with language-aware highlighting.
- Markdown remains safe with `html: false` unchanged.

### 3) Light-mode code styling and fenced/inline separation

- File: `web/src/styles.css`
- Updated `.message-prose pre` and `.message-prose pre code` for clearer block surface and typography.
- Scoped inline code to `.message-prose :not(pre) > code` so fenced blocks are not double-styled.
- Added `.message-prose .hljs*` token color rules aligned with approved palette tokens/derivatives.
- Final theme direction:
  - light code block surface (`bg-input`)
  - foreground text and primary-accent token emphasis

Result:

- Fenced blocks are easier to scan.
- Inline code remains compact and visually distinct.
- Token colors stay consistent with project color constraints.

## Verification

Run in `web/`:

1. `bun run typecheck`
2. `bun run build`

Both passed after changes.

## Acceptance Criteria

1. Fenced code blocks render with syntax highlighting in chat messages.
2. Unknown or missing fenced language still highlights via auto-detect fallback.
3. Inline code keeps separate styling from fenced code.
4. Light-mode code block appearance is readable and consistent with app tokens.
5. Web typecheck and build pass.

## Follow-ups (Optional)

1. Add code-block header with detected/declared language label.
2. Add copy-to-clipboard action for fenced blocks.
3. Add rendering fixtures/snapshots for representative code samples (TS/JSON/YAML/Shell).

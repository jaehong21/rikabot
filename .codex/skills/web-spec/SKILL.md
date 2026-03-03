---
name: web-spec
description: Enforce the project web UI specification for colors, typography, separator styling, and page routing/query-state rules. Use when creating or editing frontend UI/CSS/Tailwind/router behavior in this repository to keep visual and navigation output consistent with the locked standards.
---

# Web Spec

## Apply Palette

- Use only these colors:
  - Background: `#FAF9F5`
  - Text and dark action surface: `#141414`
  - Input surface and inverse text: `#FFFFFF`
  - Primary (e.g. accent/action): `#C6603F`
  - Separator: `#D9D7D4`
  - User chat bubble surface: `#F0EEE6`
  - Settings switch enabled track: `#3A8DDE` (approved settings toggle exception)
- Use opacity variants of the approved colors when needed.
- Do not introduce additional colors beyond this list unless explicitly approved by the user.

### Allowed Alpha/Effects

- Shadows/noise/gradients must be derived only from approved palette RGB values.
- Allowed examples in this project:
  - `rgba(20, 20, 20, <alpha>)`
  - `rgba(198, 96, 63, <alpha>)`
  - `rgba(58, 141, 222, <alpha>)`
  - `rgba(255, 255, 255, <alpha>)`
  - `rgba(250, 249, 245, <alpha>)`
- Do not add new hue families for effects.

## Apply Typography

- Use this font stack everywhere:
  - `"Fira Code", "Fira Mono", Menlo, Consolas, "DejaVu Sans Mono", monospace`
- Keep body text and controls on this stack unless the user explicitly requests otherwise.

## Implementation Rules

- Keep theme tokens centralized (for this project: `web/src/styles.css` and `web/tailwind.config.ts`).
- Consume theme tokens from components instead of hardcoding ad-hoc colors.
- Use separator color `#D9D7D4` with `0.5px` thickness unless user overrides.
- Required token coverage for current UI:
  - `--background`, `--foreground`, `--input`, `--primary`, `--border`, `--user-bubble`, `--switch-on`
- For dark primary action buttons (e.g. "Always allow"), use foreground surface with inverse text:
  - background: `#141414`
  - text: `#FFFFFF`
- For secondary/outline actions, use separator border and transparent/input backgrounds.
- Replace non-compliant colors with token-based equivalents when touching UI files.
- For de-emphasized metadata text (e.g. tool call labels, summaries, helper text), prefer `opacity-50` on foreground/muted text instead of introducing new gray colors.

### Settings Switch UI

- Settings toggle track size: `44x24` (`h-6 w-11`) with full rounding.
- Enabled track color: `#3A8DDE`.
- Disabled track color: separator-derived neutral (`#D9D7D4` family).
- Thumb uses input white (`#FFFFFF`) with subtle border.
- Do not use shadows on settings toggle track or thumb.

### Route Surface Consistency

- Chat route (`/`) should use a flat `background` token surface for header and main canvas.
- If decorative overlays (noise/gradient) are used, keep them off the chat route unless explicitly requested.

## Page Routing Rules

- Keep route definitions centralized in `web/src/router.tsx`; do not introduce ad-hoc route constants in component files.
- Use TanStack Router navigation APIs (`useNavigate`, route `search` state) instead of `window.location` mutations.
- Keep current canonical pages stable unless explicitly requested:
  - `/` (chat)
  - `/settings`
  - `/threads`
- For settings sections, use query-driven state only on `/settings`:
  - Canonical form: `/settings?section=<id>`
  - Allowed section ids: `general`, `permissions`, `skills`, `mcp`
  - Invalid/missing `section` must resolve to `general`
- When adding query-backed UI state, make URL query the source of truth so refresh/back-forward preserves state.
- Validate and normalize route search params in router-level `validateSearch` rather than scattered component parsing.
- Do not use hash fragments (`#...`) for settings subsection selection when query params are already the contract.
- Keep navigation payloads typed and minimal (`navigate({ to, search })`), avoiding unrelated query key churn.

## Verification Checklist

- Search for disallowed color literals and utility colors before finalizing.
- Confirm separators use the approved color and thin stroke.
- Confirm newly touched components consume theme tokens (no ad-hoc inline color styles).
- Confirm de-emphasized gray text uses `opacity-50` where applicable.
- For routing-related UI changes, confirm:
  - `/settings?section=skills` opens Skills section
  - refresh preserves active section
  - browser back/forward restores section state
- Run checks after UI edits:
  - `cd web && bun run typecheck`
  - `cd web && bun run build`

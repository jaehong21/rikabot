---
name: web-spec
description: Enforce the project web UI specification for colors, typography, and separator styling. Use when creating or editing frontend UI/CSS/Tailwind in this repository to keep visual output consistent with the locked palette and font rules.
---

# Web Spec

## Apply Palette

- Use only these colors:
  - Background: `#FAF9F5`
  - Primary (e.g. button): `#C6603F`
  - Input surface: `#FFFFFF`
  - Text: `#141414`
  - Separator: `#D9D7D4`
- Use opacity variants of these colors when needed.
- Do not introduce additional colors. Ask the user before adding any new color.

## Apply Typography

- Use this font stack everywhere:
  - `"Fira Code", "Fira Mono", Menlo, Consolas, "DejaVu Sans Mono", monospace`
- Keep body text and controls on this stack unless the user explicitly requests otherwise.

## Implementation Rules

- Keep theme tokens centralized (for this project: `web/src/styles.css` and `web/tailwind.config.ts`).
- Consume theme tokens from components instead of hardcoding ad-hoc colors.
- Use separator color `#D9D7D4` with `0.5px` thickness unless user overrides.
- Replace non-compliant colors with token-based equivalents when touching UI files.

## Verification Checklist

- Search for disallowed color literals and utility colors before finalizing.
- Confirm separators use the approved color and thin stroke.
- Run checks after UI edits:
  - `cd web && bun run typecheck`
  - `cd web && bun run build`

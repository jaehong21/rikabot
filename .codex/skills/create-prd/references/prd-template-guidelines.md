# PRD Template Guidelines

Use this checklist whenever generating or completing a PRD for substantial changes.

## Must-have structure

- `Context`: Why now, and what problem this change solves.
- `Goals`: Concrete outcomes tied to user/business impact.
- `Non-Goals`: Prevent scope creep with explicit exclusions.
- `Requirements`: Separate functional and non-functional requirements.
- `Architecture and Design Impact`: Show how parts connect and what changes.
- `Implementation Plan`: Ordered phases, not just a task list.
- `File-by-File Changes`: Point to explicit files to reduce ambiguity.
- `Testing and Validation`: How to verify behavior before merge.
- `Acceptance Criteria`: Measurable and reviewable items.
- `Risks and Mitigations`: At least 2 risks for non-trivial changes.

## Quality bar

- Replace every `TODO` before handoff unless the user asked for a draft.
- Every requirement should be traceable to at least one file/API/flag in the repo.
- Use explicit verification criteria (`command`, `test`, or `manual check`).
- Include compatibility and migration notes for behavior changes.
- Keep scope bounded with clear assumptions and dependencies.

## For docs/prd consistency

Match the repo style in existing PRDs:

- Section names are concise and imperative.
- Include numbered acceptance criteria.
- Mention command-level verification where applicable.
- Include a short risk section for operational and security impact.

## Good phrasing patterns

- "### Functional Requirements" and "### Non-Functional Requirements"
- "### Phase 1 / Phase 2 / Phase 3" under implementation
- "### Risks and Mitigations" with paired bullets
- "## Open Questions" if decisions remain unresolved

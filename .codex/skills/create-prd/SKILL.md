---
name: create-prd
description: Use for creating and drafting large-scope Product Requirements Documents under docs/prd for feature, architecture, and systems changes that need context, requirements, implementation plans, and acceptance criteria.
---

# Create PRD

## Overview

Draft Product Requirements Documents (PRDs) for substantial changes. Use this when work affects multiple files, introduces behavior changes, or requires coordinated implementation and validation decisions.

## Workflow

1. Confirm the change is a large-scope item (multiple components, user-facing behavior, migration, infra, or security impact).
2. Generate a skeleton with `scripts/create-prd.py` or start from an existing PRD style.
3. Fill sections in this order: Context, Goals, Non-Goals, Requirements, Architecture/Design Impact, Implementation Plan, Validation, Acceptance Criteria, Risks.
4. Include concrete file paths, flags, env vars, and tests for every major behavior change.
5. Ensure at least one measurable acceptance test and one rollback/compatibility check before finalizing.

## Scope Selection

- Use this skill when changes are broad, cross-cutting, or high-risk.
- Prefer a short note for trivial edits (single-file refactor, typo fixes, minor copy changes).

## Script: create-prd

Use the generator to create a correctly numbered PRD file with a reusable scaffold.

```bash
python .codex/skills/create-prd/scripts/create-prd.py --title "Your PRD title" [--id 0001] [--docs-dir docs/prd]
```

### Script behavior

- Auto-discovers next PRD number from `docs/prd` when `--id` is omitted.
- Sanitizes title into a file-safe slug and truncates it to keep PRD filenames short.
- Includes section skeleton aligned with repo PRD patterns.
- Uses a zero-padded 4-digit PRD identifier by default.

## Reference

- Follow the section quality rules in [`references/prd-template-guidelines.md`](references/prd-template-guidelines.md).
- Use the generated file as a baseline and rewrite section content for this feature before saving.

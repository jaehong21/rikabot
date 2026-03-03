#!/usr/bin/env python3
"""Create a PRD scaffold in docs/prd.

Usage:
  python create-prd.py --title "Your PRD title"
  python create-prd.py --title "..." --id 0008
  python create-prd.py --title "..." --docs-dir docs/prd --overwrite
"""

from __future__ import annotations

import argparse
import re
from pathlib import Path

MAX_SLUG_LEN = 60


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Create a PRD markdown scaffold")
    parser.add_argument("--title", required=True, help="PRD title")
    parser.add_argument(
        "--docs-dir",
        default="docs/prd",
        help="Directory containing PRD files (default: docs/prd)",
    )
    parser.add_argument(
        "--id",
        default=None,
        help="Optional 4-digit PRD id. If omitted, auto-increments from existing docs/prd files.",
    )
    parser.add_argument(
        "--overwrite",
        action="store_true",
        help="Allow replacing existing target PRD file.",
    )
    return parser.parse_args()


def safe_slug(value: str) -> str:
    slug = re.sub(r"[^a-zA-Z0-9]+", "-", value.strip().lower())
    slug = slug.strip("-")
    slug = re.sub(r"-+", "-", slug)
    slug = slug[:MAX_SLUG_LEN].strip("-")
    return slug or "prd"


def next_prd_id(docs_dir: Path) -> int:
    ids: list[int] = []
    for candidate in docs_dir.glob("[0-9][0-9][0-9][0-9]-*.md"):
        match = re.match(r"^(\d{4})-", candidate.name)
        if match:
            ids.append(int(match.group(1)))
    return max(ids) + 1 if ids else 1


def build_content(title: str) -> str:
    return f"""# {title}

## Context

- TODO: Explain why this change is needed now and what pain it resolves.

## Goals

- TODO: Describe desired outcomes with measurable success signals.

## Non-Goals

- TODO: Document explicit exclusions for this PRD scope.

## Requirements

### Functional Requirements

- TODO: Add requirement bullets with user-visible behavior.

### Non-Functional Requirements

- TODO: Add reliability, performance, and operational constraints.

## Architecture and Design Impact

- TODO: Explain component interactions, data flow, and design tradeoffs.

## Implementation Plan

### Phase 1: Setup

- TODO: Describe preparatory work.

### Phase 2: Core Work

- TODO: Describe implementation steps and dependencies.

### Phase 3: Validation

- TODO: Describe migration, smoke tests, and rollback checks.

## File-by-File Changes

- TODO: List target files and expected edits.

## Testing and Validation

- TODO: Add unit/integration/e2e checks.

## Acceptance Criteria

1. TODO: Define concrete acceptance criteria.
2. TODO: Define measurable behavior for each major subsystem.

## Risks and Mitigations

- **Risk**: TODO
  - **Impact**: TODO
  - **Mitigation**: TODO

## Open Questions

- TODO: List unresolved decisions and owners.
"""


def main() -> int:
    args = parse_args()
    title = args.title.strip()
    docs_dir = Path(args.docs_dir)

    if not title:
        raise SystemExit("--title cannot be empty.")

    if not docs_dir.exists():
        raise SystemExit(f"docs directory does not exist: {docs_dir}")

    if args.id is None:
        prd_num = next_prd_id(docs_dir)
    else:
        id_match = re.fullmatch(r"\d{4}", args.id.strip())
        if not id_match:
            raise SystemExit("--id must be exactly four digits (e.g. 0009)")
        prd_num = int(args.id)

    prd_id = f"{prd_num:04d}"
    slug = safe_slug(title)
    output_path = docs_dir / f"{prd_id}-{slug}.md"

    if output_path.exists() and not args.overwrite:
        raise SystemExit(
            f"PRD already exists: {output_path}\nUse --overwrite if this is intentional."
        )

    docs_dir.mkdir(parents=True, exist_ok=True)
    output_path.write_text(build_content(title), encoding="utf-8")

    print(f"Created PRD: {output_path}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

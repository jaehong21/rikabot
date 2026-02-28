# TOOLS.md - Local Notes

Use this file for workspace-specific operational details that are not universal tool behavior.

## What belongs here

- Hostnames, aliases, SSH targets
- Repo or environment conventions
- Repeated command patterns
- Local paths and service names

## Example

```markdown
## SSH
- prod-api -> 10.0.0.12 (read-only diagnostics)

## Commands
- test: `cargo test`
- lint: `cargo clippy --all-targets --all-features`
```

Keep entries brief and factual.

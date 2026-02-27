---
name: github
description: "GitHub operations via `gh` CLI: issues, PRs, CI runs, code review, API queries, and fetching content from GitHub URLs (commits, files, trees, comparisons). Use when: (1) checking PR status or CI, (2) creating/commenting on issues, (3) listing/filtering PRs or issues, (4) viewing run logs, (5) fetching content from GitHub URLs (commit, blob, tree, compare). NOT for: complex web UI interactions requiring manual browser flows (use browser tooling when available), bulk operations across many repos (script with gh api), or when gh auth is not configured."
requires:
  bins: ["gh"]
---

# GitHub Skill

Use the `gh` CLI to interact with GitHub repositories, issues, PRs, CI, and to fetch content from GitHub URLs.

## When to Use

✅ **USE this skill when:**

- Checking PR status, reviews, or merge readiness
- Viewing CI/workflow run status and logs
- Creating, closing, or commenting on issues
- Creating or merging pull requests
- Querying GitHub API for repository data
- Listing repos, releases, or collaborators
- Fetching content from GitHub URLs (commits, files, directories, comparisons)

## When NOT to Use

❌ **DON'T use this skill when:**

- Local git operations (commit, push, pull, branch) → use `git` directly
- Non-GitHub repos (GitLab, Bitbucket, self-hosted) → different CLIs
- Cloning repositories → use `git clone`
- Reviewing actual code changes → use `coding-agent` skill
- Complex multi-file diffs → use `coding-agent` or read files directly

## Setup

```bash
# Authenticate (one-time)
gh auth login

# Verify
gh auth status
```

## Common Commands

### Pull Requests

```bash
# List PRs
gh pr list --repo owner/repo

# Check CI status
gh pr checks 55 --repo owner/repo

# View PR details
gh pr view 55 --repo owner/repo

# Create PR
gh pr create --title "feat: add feature" --body "Description"

# Merge PR
gh pr merge 55 --squash --repo owner/repo
```

### Issues

```bash
# List issues
gh issue list --repo owner/repo --state open

# Create issue
gh issue create --title "Bug: something broken" --body "Details..."

# Close issue
gh issue close 42 --repo owner/repo
```

### CI/Workflow Runs

```bash
# List recent runs
gh run list --repo owner/repo --limit 10

# View specific run
gh run view <run-id> --repo owner/repo

# View failed step logs only
gh run view <run-id> --repo owner/repo --log-failed

# Re-run failed jobs
gh run rerun <run-id> --failed --repo owner/repo
```

### API Queries

```bash
# Get PR with specific fields
gh api repos/owner/repo/pulls/55 --jq '.title, .state, .user.login'

# List all labels
gh api repos/owner/repo/labels --jq '.[].name'

# Get repo stats
gh api repos/owner/repo --jq '{stars: .stargazers_count, forks: .forks_count}'
```

## JSON Output

Most commands support `--json` for structured output with `--jq` filtering:

```bash
gh issue list --repo owner/repo --json number,title --jq '.[] | "\(.number): \(.title)"'
gh pr list --json number,title,state,mergeable --jq '.[] | select(.mergeable == "MERGEABLE")'
```

## Templates

### PR Review Summary

```bash
# Get PR overview for review
PR=55 REPO=owner/repo
echo "## PR #$PR Summary"
gh pr view $PR --repo $REPO --json title,body,author,additions,deletions,changedFiles \
  --jq '"**\(.title)** by @\(.author.login)\n\n\(.body)\n\n📊 +\(.additions) -\(.deletions) across \(.changedFiles) files"'
gh pr checks $PR --repo $REPO
```

### Issue Triage

```bash
# Quick issue triage view
gh issue list --repo owner/repo --state open --json number,title,labels,createdAt \
  --jq '.[] | "[\(.number)] \(.title) - \([.labels[].name] | join(", ")) (\(.createdAt[:10]))"'
```

## Fetching Content from GitHub URLs

When a user provides a GitHub URL, parse it to extract the owner, repo, and relevant path components, then use `gh api` to fetch the content.

### URL Parsing Guide

GitHub URLs follow predictable patterns. Extract components as follows:

| URL Pattern                                           | Components                                   |
| ----------------------------------------------------- | -------------------------------------------- |
| `https://github.com/owner/repo/commit/SHA`            | owner, repo, SHA                             |
| `https://github.com/owner/repo/blob/REF/path/to/file` | owner, repo, ref (branch/tag/SHA), file path |
| `https://github.com/owner/repo/tree/REF/path/to/dir`  | owner, repo, ref, directory path             |
| `https://github.com/owner/repo/compare/BASE...HEAD`   | owner, repo, base ref, head ref              |
| `https://github.com/owner/repo/pull/NUMBER`           | owner, repo, PR number                       |
| `https://github.com/owner/repo/issues/NUMBER`         | owner, repo, issue number                    |

When parsing, strip any query parameters or fragment identifiers. The `REF` in blob/tree URLs can be a branch name, tag, or commit SHA.

### Fetching Commits

```bash
# View commit message and changed files
gh api repos/owner/repo/commits/abc123def \
  --jq '"Message: \(.commit.message)\nAuthor: \(.commit.author.name)\nDate: \(.commit.author.date)\n\nFiles changed:\n\(.files[] | "  \(.status) \(.filename)")"'

# Get just the file list from a commit
gh api repos/owner/repo/commits/abc123def --jq '.files[].filename'

# Get the full patch/diff for a commit
gh api repos/owner/repo/commits/abc123def --jq '.files[] | "--- \(.filename) ---\n\(.patch)\n"'
```

### Fetching File Contents (blob URLs)

```bash
# Get raw file content (best for text files)
gh api repos/owner/repo/contents/path/to/file.yaml?ref=main \
  -H "Accept: application/vnd.github.raw"

# Get base64-encoded content and decode it
gh api repos/owner/repo/contents/path/to/file.yaml?ref=main \
  --jq '.content' | base64 -d

# Get file metadata (size, SHA, download URL)
gh api repos/owner/repo/contents/path/to/file.yaml?ref=main \
  --jq '{name: .name, size: .size, sha: .sha, download_url: .download_url}'
```

The `?ref=` parameter specifies the branch, tag, or commit SHA. If omitted, the repo's default branch is used.

### Fetching Directory Listings (tree URLs)

```bash
# List files in a directory
gh api repos/owner/repo/contents/path/to/dir?ref=main \
  --jq '.[] | "\(.type)\t\(.name)"'

# List only file names
gh api repos/owner/repo/contents/path/to/dir?ref=main --jq '.[].name'

# Get detailed listing with sizes
gh api repos/owner/repo/contents/path/to/dir?ref=main \
  --jq '.[] | "\(.type)\t\(.size)\t\(.name)"'

# Recursive tree listing (for deeper traversal)
gh api repos/owner/repo/git/trees/main?recursive=1 \
  --jq '.tree[] | select(.path | startswith("path/to/dir/")) | "\(.type)\t\(.path)"'
```

### Fetching Comparisons (compare URLs)

```bash
# Compare two refs and list commits
gh api repos/owner/repo/compare/main...feature-branch \
  --jq '.commits[] | "\(.sha[:8]) \(.commit.message | split("\n")[0])"'

# Show files changed between two refs
gh api repos/owner/repo/compare/main...feature-branch \
  --jq '.files[] | "\(.status)\t\(.filename)"'

# Get comparison summary
gh api repos/owner/repo/compare/main...feature-branch \
  --jq '"Ahead by \(.ahead_by) commits, behind by \(.behind_by) commits\nTotal files changed: \(.files | length)\nAdditions: \([.files[].additions] | add)\nDeletions: \([.files[].deletions] | add)"'

# Get the patch for changed files in a comparison
gh api repos/owner/repo/compare/main...feature-branch \
  --jq '.files[] | "--- \(.filename) ---\n\(.patch)\n"'
```

### URL-to-Command Quick Reference

When a user gives you a GitHub URL, map it to a `gh` command:

```
https://github.com/owner/repo/commit/SHA
  → gh api repos/owner/repo/commits/SHA

https://github.com/owner/repo/blob/REF/path/to/file
  → gh api repos/owner/repo/contents/path/to/file?ref=REF -H "Accept: application/vnd.github.raw"

https://github.com/owner/repo/tree/REF/path/to/dir
  → gh api repos/owner/repo/contents/path/to/dir?ref=REF

https://github.com/owner/repo/compare/BASE...HEAD
  → gh api repos/owner/repo/compare/BASE...HEAD

https://github.com/owner/repo/pull/NUMBER
  → gh pr view NUMBER --repo owner/repo

https://github.com/owner/repo/issues/NUMBER
  → gh issue view NUMBER --repo owner/repo
```

## Notes

- Always specify `--repo owner/repo` when not in a git directory
- Use URLs directly: `gh pr view https://github.com/owner/repo/pull/55`
- Rate limits apply; use `gh api --cache 1h` for repeated queries
- When fetching file contents, large files (>1MB) may require the Git Blobs API: `gh api repos/owner/repo/git/blobs/SHA`
- The Contents API returns base64-encoded content by default; use the `Accept: application/vnd.github.raw` header for raw text

#!/usr/bin/env bash

# Release helper for rikabot:
# 1) create a git tag and GitHub release, and
# 2) publish the Cargo crate.
set -euo pipefail

usage() {
	cat <<'EOF'
Usage: ./scripts/release.sh [--tag <tag>] [--skip-github] [--skip-cargo] [--dry-run]

Options:
  --tag <tag>       Release tag to create (defaults to v<crate version> from Cargo.toml)
  --allow-dirty     Permit running with uncommitted changes
  --skip-github     Skip creating/pushing GitHub release
  --skip-cargo      Skip cargo publish
  --dry-run         Print commands without executing
  -h, --help        Show this message

Examples:
  ./scripts/release.sh
  ./scripts/release.sh --tag v1.2.3
EOF
}

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

TAG_INPUT=""
SKIP_GITHUB=0
SKIP_CARGO=0
DRY_RUN=0
ALLOW_DIRTY=0

while [[ $# -gt 0 ]]; do
	case "$1" in
		--tag)
			TAG_INPUT="${2:-}"
			shift 2
			;;
		--allow-dirty)
			ALLOW_DIRTY=1
			shift
			;;
		--skip-github)
			SKIP_GITHUB=1
			shift
			;;
		--skip-cargo)
			SKIP_CARGO=1
			shift
			;;
		--dry-run)
			DRY_RUN=1
			shift
			;;
		-h|--help)
			usage
			exit 0
			;;
		*)
			echo "Unknown argument: $1"
			usage
			exit 1
			;;
	esac
done

require_cmd() {
	local cmd="$1"
	if ! command -v "$cmd" > /dev/null; then
		echo "Missing required command: $cmd" >&2
		exit 1
	fi
}

run() {
	if [[ "$DRY_RUN" == "1" ]]; then
		echo "[dry-run] $*"
	else
		"$@"
	fi
}

require_cmd git
require_cmd cargo

if ! git rev-parse --is-inside-work-tree > /dev/null 2>&1; then
	echo "Not a git repository: $ROOT_DIR" >&2
	exit 1
fi

if [[ "$ALLOW_DIRTY" == "0" && -n "$(git status --short)" ]]; then
	echo "Working tree is dirty. Commit or stash changes first." >&2
	exit 1
fi

if [[ ! -f Cargo.toml ]]; then
	echo "Cargo.toml not found at repository root." >&2
	exit 1
fi

VERSION="$(sed -n 's/^version *= *"\([^"]*\)".*/\1/p' Cargo.toml | head -n 1)"
if [[ -z "$VERSION" ]]; then
	echo "Could not read crate version from Cargo.toml." >&2
	exit 1
fi

if [[ -z "$TAG_INPUT" ]]; then
	RELEASE_TAG="v${VERSION}"
else
	RELEASE_TAG="$TAG_INPUT"
	[[ "$RELEASE_TAG" != v* ]] && RELEASE_TAG="v${RELEASE_TAG}"
fi

if [[ "$RELEASE_TAG" != "v${VERSION}" ]]; then
	echo "Warning: tag ${RELEASE_TAG} does not match Cargo.toml version v${VERSION}."
	echo "         This is allowed, but usually tags should reflect the crate version."
fi

if git show-ref --tags --verify --quiet "refs/tags/${RELEASE_TAG}"; then
	echo "Tag ${RELEASE_TAG} already exists. Aborting." >&2
	exit 1
fi

if [[ "$SKIP_GITHUB" == "0" ]]; then
	require_cmd gh
fi

echo "Preparing release ${RELEASE_TAG} (crate version: ${VERSION})"

# Generate release notes from commits since previous version tag.
LATEST_TAG=""
while IFS= read -r tag; do
	if [[ "$tag" != "$RELEASE_TAG" ]]; then
		LATEST_TAG="$tag"
	fi
done < <(git tag --list 'v*' --sort=version:refname)
NOTES_FILE="$(mktemp)"
trap 'rm -f "$NOTES_FILE"' EXIT

if [[ -n "$LATEST_TAG" ]]; then
	git log --pretty=format:"- %s (%h)" "${LATEST_TAG}..HEAD" > "$NOTES_FILE"
else
	git log --pretty=format:"- %s (%h)" --max-count=50 > "$NOTES_FILE"
fi
if [[ ! -s "$NOTES_FILE" ]]; then
	echo "Release notes for this tag are empty. Adding default note." > "$NOTES_FILE"
fi

if [[ "$SKIP_GITHUB" == "0" ]]; then
	run gh auth status
	run git tag "$RELEASE_TAG" -m "Release ${RELEASE_TAG}"
	run git push origin "$RELEASE_TAG"
	run gh release create "$RELEASE_TAG" --title "$RELEASE_TAG" --notes-file "$NOTES_FILE"
fi

if [[ "$SKIP_CARGO" == "0" ]]; then
	run cargo publish --locked
fi

echo "Release workflow complete for ${RELEASE_TAG}."

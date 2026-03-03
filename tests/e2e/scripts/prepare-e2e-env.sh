#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
TMP_DIR="${ROOT_DIR}/.tmp/e2e"
WORKSPACE_DIR="${TMP_DIR}/workspace"
CONFIG_DIR="${TMP_DIR}/config"
CONFIG_PATH="${CONFIG_DIR}/rika.e2e.toml"
TEMPLATE_PATH="${ROOT_DIR}/tests/e2e/config/rika.e2e.template.toml"

rm -rf "${WORKSPACE_DIR}" "${TMP_DIR}/playwright"
mkdir -p "${WORKSPACE_DIR}" "${CONFIG_DIR}" "${TMP_DIR}/playwright"
cp "${TEMPLATE_PATH}" "${CONFIG_PATH}"

SKILL_DIR="${WORKSPACE_DIR}/skills/e2e-skill"
mkdir -p "${SKILL_DIR}"
cat > "${SKILL_DIR}/SKILL.md" <<'EOF'
---
name: e2e-skill
description: Skill seeded for frontend E2E tests
always: false
---

# E2E Skill

This skill file is created by the Playwright test bootstrap.
EOF

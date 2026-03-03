#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
CONFIG_PATH="${ROOT_DIR}/.tmp/e2e/config/rika.e2e.toml"

if [[ ! -f "${CONFIG_PATH}" ]]; then
  bash "${ROOT_DIR}/tests/e2e/scripts/prepare-e2e-env.sh"
fi

exec cargo run -- --config "${CONFIG_PATH}" start --foreground

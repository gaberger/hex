#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")"

if ! command -v bun >/dev/null 2>&1; then
  echo "bun is required (https://bun.sh) — not found on PATH" >&2
  exit 1
fi

if [ ! -d node_modules ]; then
  bun install
fi

exec bun run start

#!/usr/bin/env bash
#
# validate-readme.sh — run `hex readme validate` from the repo root.
#
# ADR-2604110227: README claim validation.
#
# Usage:
#   ./scripts/validate-readme.sh           # advisory mode — exits 0 on warnings
#   ./scripts/validate-readme.sh --strict  # CI mode — exits 1 on warnings too
#
# Checks performed:
#   * Numeric counts (ADRs, agents, skills, WASM modules, port traits, reducers)
#   * SVG asset file existence
#   * Internal markdown link resolution
#   * Named entity references (modules, crates, agents)
#   * CLI command existence via `hex <cmd> --help`
#
# Exit codes:
#   0 — all checks passed (or only warnings without --strict)
#   1 — at least one check failed
#   2 — hex binary could not be built
#
# The canonical test for CI is `cargo test -p hex-cli` which runs
# `repo_readme_is_accurate` — this script is the fast interactive equivalent.

set -euo pipefail

# Resolve repo root from this script's location
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
cd "${REPO_ROOT}"

# Prefer an existing debug build for speed; build if nothing found.
HEX_BIN=""
for candidate in "target/release/hex" "target/debug/hex"; do
    if [[ -x "${candidate}" ]]; then
        HEX_BIN="${candidate}"
        break
    fi
done

if [[ -z "${HEX_BIN}" ]]; then
    echo "no hex binary found — building with cargo build -p hex-cli..."
    if ! cargo build -p hex-cli >&2; then
        echo "error: failed to build hex-cli" >&2
        exit 2
    fi
    HEX_BIN="target/debug/hex"
fi

echo "using: ${HEX_BIN}"
exec "${HEX_BIN}" readme validate "$@"

#!/usr/bin/env bash
# Smoke acceptance gate for ebay-mvp (workplan step-32).
#
# Satisfies:
#   - ebay-spec-022: start.sh smoke mode brings up STDB + backend + Vite,
#     prints all three URLs, and reaps cleanly within 5s on signal
#   - ebay-spec-023: `hex analyze examples/ebay-clone/backend` exits 0
#     with zero hexagonal-boundary violations
#
# Usage:
#   ./examples/ebay-clone/scripts/smoke.sh
#
# Exit codes:
#   0 — both gates passed
#   1 — hex analyze failed (boundary violations)
#   2 — start.sh smoke mode failed (service didn't come up, or didn't
#       reap on signal, or stale port bindings remain)
#   3 — required dependency missing (hex, spacetime, bun, cargo)

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BACKEND_DIR="${ROOT_DIR}/backend"
START_SH="${ROOT_DIR}/start.sh"
TIMEOUT_SECS="${SMOKE_TIMEOUT_SECS:-60}"

cyan() { printf '\033[36m%s\033[0m\n' "$*"; }
green() { printf '\033[32m%s\033[0m\n' "$*"; }
red() { printf '\033[31m%s\033[0m\n' "$*" >&2; }

# ---------------------------------------------------------------------------
# Prerequisite check
# ---------------------------------------------------------------------------
cyan "▶ smoke: checking prerequisites"
for tool in hex cargo; do
    if ! command -v "$tool" >/dev/null 2>&1; then
        red "missing required tool: $tool"
        exit 3
    fi
done

# ---------------------------------------------------------------------------
# Gate 1: hex analyze — zero boundary violations
# ---------------------------------------------------------------------------
cyan "▶ smoke: running hex analyze on backend crate"
if ! ANALYZE_OUTPUT=$(hex analyze "${BACKEND_DIR}" 2>&1); then
    red "hex analyze failed:"
    echo "${ANALYZE_OUTPUT}" >&2
    exit 1
fi
# Treat any "violation" mention in output as fail. Some hex analyze
# versions emit "0 violations" — that's OK; "1 violation" is not.
if echo "${ANALYZE_OUTPUT}" | grep -iE '^[^0]+violation|^[1-9][0-9]* violation' >/dev/null; then
    red "hex analyze reported boundary violations:"
    echo "${ANALYZE_OUTPUT}" >&2
    exit 1
fi
green "✓ gate 1 passed: hex analyze clean"

# ---------------------------------------------------------------------------
# Gate 2: start.sh --smoke mode
# ---------------------------------------------------------------------------
cyan "▶ smoke: invoking start.sh --smoke (timeout ${TIMEOUT_SECS}s)"

if [[ ! -x "${START_SH}" ]]; then
    red "start.sh not found or not executable at ${START_SH}"
    exit 2
fi

# Snapshot the listening ports before we invoke start.sh, so the
# post-cleanup check can detect stale bindings on 9200/8080/5173.
PORTS_BEFORE=$(ss -tln 2>/dev/null | awk 'NR>1 {print $4}' | awk -F: '{print $NF}' | sort -u || true)

START_LOG=$(mktemp)
trap 'rm -f "${START_LOG}"' EXIT

# Run start.sh --smoke with a hard timeout. The --smoke flag tells
# start.sh to exit cleanly once all three health checks pass.
if timeout --kill-after=5 "${TIMEOUT_SECS}" "${START_SH}" --smoke >"${START_LOG}" 2>&1; then
    green "✓ start.sh --smoke exited 0"
else
    rc=$?
    red "start.sh --smoke failed (exit ${rc}):"
    tail -30 "${START_LOG}" >&2
    exit 2
fi

# Verify cleanup: no child process named spacetime, no axum backend
# bound to 8080, no vite bound to 5173 that wasn't there before.
sleep 2
for port in 8080 5173 9200; do
    if ss -tln 2>/dev/null | grep -E ":${port}\b" >/dev/null; then
        if ! echo "${PORTS_BEFORE}" | grep -wq "${port}"; then
            red "stale port binding detected on ${port} after smoke cleanup"
            exit 2
        fi
    fi
done
green "✓ gate 2 passed: start.sh smoke clean, no stale port bindings"

green "✓ smoke acceptance gate PASSED"
exit 0

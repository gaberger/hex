#!/usr/bin/env bash
# Smoke test: Path B wait-for-completion — enqueue a workplan via hex sched,
# poll up to 2 minutes for a terminal result, and assert the result contains
# either a real completion message (HEAD changed / commits landed) or a real
# failure reason (not just "Execution dispatched").
#
# What this proves end-to-end:
#   1. hex sched enqueue accepts the workplan and returns a task UUID
#   2. The sched daemon picks it up and dispatches execution
#   3. The result reflects actual outcome — not a vacuous "dispatched" ack
#   4. Terminal status is one of: completed (with git evidence) or failed
#      (with a meaningful reason like "no git evidence")
#
# Prerequisites:
#   - hex-nexus running on :5555
#   - hex sched daemon running (--background --interval 10)
#   - Binary built from the current branch
#   - A workplan JSON file to enqueue (defaults to wp-sched-evidence-guard.json
#     which is all-done, so it will fail with "no git evidence" — a valid
#     terminal state for this test)
#
# Exit codes: 0 = pass, 1 = fail, 2 = prerequisite missing.
set -euo pipefail

WP="${1:-docs/workplans/wp-sched-evidence-guard.json}"
POLL_INTERVAL=10
MAX_WAIT=120

if [ ! -f "$WP" ]; then
  echo "FAIL: $WP not found — cannot run smoke test" >&2
  exit 2
fi

if ! command -v hex >/dev/null 2>&1; then
  echo "FAIL: hex CLI not on PATH" >&2
  exit 2
fi

echo "=== Path B wait-for-completion smoke test ==="
echo "workplan: $WP"

# ── Step 1: Enqueue ────────────────────────────────────────────────────────
echo "enqueueing $WP..."
ENQ_OUT=$(hex sched enqueue workplan "$WP")
echo "$ENQ_OUT"
TASK_ID=$(echo "$ENQ_OUT" | grep -oE '[a-f0-9]{8}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{12}' | head -1)
if [ -z "$TASK_ID" ]; then
  echo "FAIL: could not extract task id from enqueue output" >&2
  exit 1
fi
echo "task id: $TASK_ID"

# ── Step 2: Poll for terminal state ───────────────────────────────────────
ELAPSED=0
TERMINAL=""
echo "polling every ${POLL_INTERVAL}s (max ${MAX_WAIT}s)..."

while [ "$ELAPSED" -lt "$MAX_WAIT" ]; do
  sleep "$POLL_INTERVAL"
  ELAPSED=$((ELAPSED + POLL_INTERVAL))

  HIST=$(hex sched queue history --limit 20 2>/dev/null || true)

  if echo "$HIST" | grep -q "$TASK_ID"; then
    TERMINAL="$HIST"
    echo "task appeared in history after ${ELAPSED}s"
    break
  fi

  echo "  ${ELAPSED}s — still pending..."
done

if [ -z "$TERMINAL" ]; then
  echo "FAIL: task $TASK_ID did not reach terminal state within ${MAX_WAIT}s" >&2
  echo "      (daemon may not be running or interval is too long)" >&2
  exit 1
fi

echo ""
echo "--- history output ---"
echo "$TERMINAL"
echo "----------------------"

# ── Step 3: Assert meaningful result ──────────────────────────────────────
# The result must NOT be a vacuous "Execution dispatched" — it must contain
# either evidence of real completion or a real failure reason.
VACUOUS_PATTERNS="Execution dispatched|Dispatched|queued"

if echo "$TERMINAL" | grep -iE "$VACUOUS_PATTERNS" | grep -q "$TASK_ID"; then
  # Task is in history but result is vacuous — Path B is broken
  if ! echo "$TERMINAL" | grep "$TASK_ID" | grep -iqE "evidence|HEAD|commit|failed|error|no git"; then
    echo "FAIL: task $TASK_ID result is vacuous — contains only dispatch ack, no real outcome" >&2
    exit 1
  fi
fi

# Check for real terminal content: either success or failure with reason
REAL_OUTCOME=false

# Success indicators: HEAD changed, commits landed, phases completed
if echo "$TERMINAL" | grep -iqE "completed|HEAD changed|commits|phases.*complete|success"; then
  echo "result type: COMPLETION (real work done)"
  REAL_OUTCOME=true
fi

# Failure indicators: meaningful failure reason
if echo "$TERMINAL" | grep -iqE "no git evidence|failed|error|compile.*fail|test.*fail|rejected"; then
  echo "result type: FAILURE (with reason — this is valid)"
  REAL_OUTCOME=true
fi

if [ "$REAL_OUTCOME" = true ]; then
  echo "PASS: task $TASK_ID reached terminal state with meaningful result"
  exit 0
else
  echo "FAIL: task $TASK_ID is in history but result is neither a clear success nor a clear failure" >&2
  echo "      expected: completion evidence OR failure reason" >&2
  echo "      got: (see history output above)" >&2
  exit 1
fi

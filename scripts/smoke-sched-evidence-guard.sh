#!/usr/bin/env bash
# Smoke test: evidence-guard (ADR-2604141400 §1 P1) correctly flips a silent-
# drain workplan to `failed` with a "no git evidence" marker in the result.
#
# What this proves end-to-end:
#   1. hex sched enqueue accepts a workplan
#   2. hex sched daemon picks it up and runs `hex plan execute`
#   3. check_evidence() sees HEAD unchanged, returns success=false
#   4. update_brain_task writes status=failed + "no git evidence" to result
#   5. GET /api/brain/queue/history exposes the failed row (P1.2)
#   6. `hex sched queue history --status failed` renders it visibly (P1.3)
#
# Prerequisites (do NOT run blindly — consumes real queue state):
#   - hex-nexus running on :5555
#   - hex sched daemon running (--background --interval 10)
#   - Binary built from commit that includes this workplan's P1.1-P1.3
#   - docs/workplans/wp-sched-evidence-guard.json present (all tasks done,
#     so plan execute does NOT produce commits → guard must fire)
#
# Exit codes: 0 = pass, 1 = fail, 2 = prerequisite missing.
set -euo pipefail

WP="docs/workplans/wp-sched-evidence-guard.json"

if [ ! -f "$WP" ]; then
  echo "FAIL: $WP not found — cannot run smoke test" >&2
  exit 2
fi

if ! command -v hex >/dev/null 2>&1; then
  echo "FAIL: hex CLI not on PATH" >&2
  exit 2
fi

echo "enqueueing $WP..."
# Capture full output; grep the UUID out. The CLI prints a single line like:
#   ⬡ enqueued brain task <uuid> (workplan: docs/workplans/wp-sched-evidence-guard.json)
ENQ_OUT=$(hex sched enqueue workplan "$WP")
echo "$ENQ_OUT"
TASK_ID=$(echo "$ENQ_OUT" | grep -oE '[a-f0-9]{8}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{12}' | head -1)
if [ -z "$TASK_ID" ]; then
  echo "FAIL: could not extract task id from enqueue output" >&2
  exit 1
fi
echo "task id: $TASK_ID"

# 40s is longer than a typical daemon tick (10s) + plan execute for an
# all-done workplan (~few seconds). If the daemon isn't running, this just
# times out harmlessly and the history check fails below.
echo "waiting 40s for drain..."
sleep 40

echo "checking history..."
OUT=$(hex sched queue history --status failed --limit 10)
echo "$OUT"

if echo "$OUT" | grep -q "no git evidence"; then
  echo "PASS: evidence guard fired as expected"
  exit 0
else
  echo "FAIL: expected 'no git evidence' marker in failed-task history output" >&2
  echo "      task $TASK_ID may still be pending, or guard did not fire" >&2
  exit 1
fi

#!/usr/bin/env bash
# Smoke test: nexus executor accepts workplans with `title` (no `name`).
# After this ships, enqueueing wp-sched-evidence-guard.json (title-only)
# should not fail with "missing field `name`".
set -euo pipefail

WP="docs/workplans/wp-sched-evidence-guard.json"
echo "enqueueing $WP..."
TASK_ID=$(hex sched enqueue workplan "$WP" | grep -oE '[a-f0-9-]{36}' | head -1)
echo "task id: $TASK_ID"
echo "waiting 90s..."
sleep 90

OUT=$(hex sched queue history --limit 1)
echo "$OUT"

if echo "$OUT" | grep -q "missing field.*name"; then
  echo "FAIL: schema alias still broken — executor wants 'name', workplan has 'title'"
  exit 1
fi
echo "PASS: executor accepted workplan with title-only schema"

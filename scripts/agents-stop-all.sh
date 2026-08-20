#!/usr/bin/env bash
# Stop all hex-agent daemons

set -euo pipefail

PID_DIR="${HOME}/.hex/agents/pids"

if [[ ! -d "$PID_DIR" ]]; then
    echo "No agent PIDs found at $PID_DIR"
    exit 0
fi

stopped=0
missing=0

echo "Stopping all agents..."

for pid_file in "$PID_DIR"/*.pid; do
    [[ ! -f "$pid_file" ]] && continue

    agent_name=$(basename "$pid_file" .pid)
    pid=$(cat "$pid_file")

    if kill -0 "$pid" 2>/dev/null; then
        kill "$pid"
        echo "  ✓ Stopped $agent_name (PID $pid)"
        ((stopped++))
    else
        echo "  ✗ $agent_name not running (stale PID $pid)"
        ((missing++))
    fi

    rm -f "$pid_file"
done

echo ""
echo "Stopped: $stopped agents"
echo "Missing: $missing agents (stale PIDs cleaned)"

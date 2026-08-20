#!/usr/bin/env bash
# Check status of all hex-agent daemons

set -euo pipefail

PID_DIR="${HOME}/.hex/agents/pids"

if [[ ! -d "$PID_DIR" ]]; then
    echo "No agent PIDs found at $PID_DIR"
    exit 0
fi

running=0
stopped=0

printf "%-30s %-10s %s\n" "AGENT" "STATUS" "PID"
printf "%-30s %-10s %s\n" "-----" "------" "---"

for pid_file in "$PID_DIR"/*.pid; do
    [[ ! -f "$pid_file" ]] && continue

    agent_name=$(basename "$pid_file" .pid)
    pid=$(cat "$pid_file")

    if kill -0 "$pid" 2>/dev/null; then
        printf "%-30s %-10s %s\n" "$agent_name" "running" "$pid"
        ((running++))
    else
        printf "%-30s %-10s %s\n" "$agent_name" "stopped" "$pid (stale)"
        ((stopped++))
    fi
done

echo ""
echo "Running: $running agents"
echo "Stopped: $stopped agents"

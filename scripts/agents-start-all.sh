#!/usr/bin/env bash
# Start all hex-agent daemons for the persona hierarchy

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
AGENT_BINARY="${PROJECT_ROOT}/target/release/hex-agent"
NEXUS_HOST="${NEXUS_HOST:-127.0.0.1}"
NEXUS_PORT="${NEXUS_PORT:-5555}"
AGENT_YMLS="${PROJECT_ROOT}/hex-cli/assets/agents/hex/hex"
PID_DIR="${HOME}/.hex/agents/pids"
LOG_DIR="${HOME}/.hex/agents/logs"

mkdir -p "$PID_DIR" "$LOG_DIR"

# Check if hex-agent binary exists
if [[ ! -x "$AGENT_BINARY" ]]; then
    echo "Error: hex-agent binary not found at $AGENT_BINARY"
    echo "Run: cargo build -p hex-agent --release"
    exit 1
fi

# Extract agent names from YAML files
echo "Starting all agents..."
started=0
skipped=0

for yaml in "$AGENT_YMLS"/*.yml; do
    [[ ! -f "$yaml" ]] && continue

    agent_name=$(basename "$yaml" .yml)
    pid_file="$PID_DIR/${agent_name}.pid"
    log_file="$LOG_DIR/${agent_name}.log"

    # Check if already running
    if [[ -f "$pid_file" ]]; then
        pid=$(cat "$pid_file")
        if kill -0 "$pid" 2>/dev/null; then
            echo "  ✓ $agent_name already running (PID $pid)"
            ((skipped++))
            continue
        fi
    fi

    # Start agent daemon
    "$AGENT_BINARY" daemon \
        --agent-id "$agent_name" \
        --nexus-host "$NEXUS_HOST" \
        --nexus-port "$NEXUS_PORT" \
        > "$log_file" 2>&1 &

    pid=$!
    echo "$pid" > "$pid_file"
    echo "  ✓ Started $agent_name (PID $pid)"
    ((started++))
done

echo ""
echo "Started: $started agents"
echo "Skipped: $skipped agents (already running)"
echo ""
echo "Logs: $LOG_DIR"
echo "PIDs: $PID_DIR"
echo ""
echo "Commands:"
echo "  ./scripts/agents-status.sh     # Check agent status"
echo "  ./scripts/agents-stop-all.sh   # Stop all agents"
echo "  tail -f $LOG_DIR/*.log         # Follow logs"

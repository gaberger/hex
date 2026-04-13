#!/usr/bin/env bash
set -euo pipefail

echo "⬡ hex E2E test runner"
echo "────────────────────"

# Use the built binary if hex isn't on PATH
HEX="${HEX_BIN:-$(command -v hex 2>/dev/null || echo ./target/release/hex)}"

# Start nexus if not running
if ! curl -sf http://127.0.0.1:5555/api/health > /dev/null 2>&1; then
    echo "Starting hex-nexus..."
    "$HEX" nexus start &
    sleep 3
    STARTED_NEXUS=true
else
    echo "hex-nexus already running"
    STARTED_NEXUS=false
fi

# Run E2E tests
"$HEX" test e2e
EXIT_CODE=$?

# Cleanup
if [ "$STARTED_NEXUS" = true ]; then
    echo "Stopping hex-nexus..."
    "$HEX" nexus stop 2>/dev/null || true
fi

exit $EXIT_CODE

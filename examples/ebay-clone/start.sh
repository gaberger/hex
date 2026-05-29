#!/bin/bash

# ADR-2026-05-19-0721

# Ensure necessary tools are installed
if ! command -v spacetime &> /dev/null || ! command -v bun &> /dev/null || ! command -v rustc &> /dev/null; then
    echo "Missing required tools: spacetime, bun, rustc"
    exit 1
fi

# Start STDB
spacetime db start &
STDB_PID=$!

# Publish marketplace
spacetime publish --config ./path/to/config.toml &
PUBLISH_PID=$!

# Start backend (port 8080)
cargo run --bin backend &
BACKEND_PID=$!

# Start Vite (port 5173)
bun dev &
VITE_PID=$!

echo "STDB: http://localhost:9200"
echo "Backend API: http://localhost:8080"
echo "Vite Frontend: http://localhost:5173"

# Function to clean up child processes
cleanup() {
    echo "Shutting down..."
    kill $STDB_PID $PUBLISH_PID $BACKEND_PID $VITE_PID
    sleep 5
    wait
    exit 0
}

# Trap SIGINT
trap cleanup SIGINT

if [ "$1" = "--smoke" ]; then
    # Wait for all children to terminate within 60s
    if ! timeout -k 5 60 wait; then
        echo "Timed out waiting for services to start"
        cleanup
        exit 1
    fi

    # Check for any remaining child processes
    if pgrep -P $$ &> /dev/null; then
        echo "Child processes are still running after health checks"
        cleanup
        exit 1
    fi

    exit 0
fi

# Wait for all children to terminate
wait

exit 0
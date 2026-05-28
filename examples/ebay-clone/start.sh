code_patch: create examples/ebay-clone/start.sh

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

# Wait for all children to terminate
wait

exit 0
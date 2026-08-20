#!/usr/bin/env bash
# Launch the hexBay demo: a seeded in-memory backend + the Solid frontend.
# No SpacetimeDB required for the demo profile — the backend ships an in-memory
# marketplace pre-seeded with a catalog. (The persistent profile uses the
# spacetime-modules/marketplace WASM module — see README.)
set -euo pipefail

ROOT="$(cd "$(dirname "$0")" && pwd)"
BACKEND_ADDR="${BACKEND_ADDR:-127.0.0.1:8080}"
FRONTEND_PORT="${FRONTEND_PORT:-4173}"

echo "▶ building backend…"
( cd "$ROOT/backend" && cargo build --release --quiet )

echo "▶ installing + building frontend…"
( cd "$ROOT/frontend" && npm install --silent && npm run build )

echo "▶ starting backend on $BACKEND_ADDR (seeded catalog)…"
BACKEND_ADDR="$BACKEND_ADDR" "$ROOT/backend/target/release/main" &
BACKEND_PID=$!

echo "▶ serving frontend on :$FRONTEND_PORT…"
( cd "$ROOT/frontend" && npx vite preview --port "$FRONTEND_PORT" --host 127.0.0.1 ) &
FRONTEND_PID=$!

trap 'kill "$BACKEND_PID" "$FRONTEND_PID" 2>/dev/null || true' EXIT
echo ""
echo "  ✅ hexBay running →  http://localhost:$FRONTEND_PORT   (API: http://$BACKEND_ADDR)"
echo "     Ctrl-C to stop."
wait

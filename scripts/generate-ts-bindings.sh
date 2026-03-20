#!/usr/bin/env bash
# generate-ts-bindings.sh — Generate TypeScript client bindings from SpacetimeDB WASM modules
# Usage: ./scripts/generate-ts-bindings.sh
#
# Prerequisites:
#   - spacetime CLI installed (https://spacetimedb.com/install)
#   - WASM modules compiled: cd spacetime-modules && cargo build --release --target wasm32-unknown-unknown

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
MODULES_DIR="$PROJECT_ROOT/spacetime-modules"
OUT_BASE="$PROJECT_ROOT/hex-chat/ui/src/spacetimedb"
OUT_DASHBOARD="$PROJECT_ROOT/hex-nexus/assets/src/spacetimedb"

# All modules to generate bindings for (priority order)
MODULES=(
  hexflo-coordination
  agent-registry
  chat-relay
  inference-gateway
  fleet-state
  hexflo-cleanup
  hexflo-lifecycle
  inference-bridge
)

# Check prerequisites
if ! command -v spacetime &>/dev/null; then
  echo "ERROR: spacetime CLI not found. Install from https://spacetimedb.com/install"
  exit 1
fi

if [ ! -d "$MODULES_DIR" ]; then
  echo "ERROR: spacetime-modules/ directory not found at $MODULES_DIR"
  exit 1
fi

# Build WASM modules if not already built
echo "==> Checking WASM build..."
WASM_TARGET="$MODULES_DIR/target/wasm32-unknown-unknown/release"
NEEDS_BUILD=false
for mod in "${MODULES[@]}"; do
  wasm_name="${mod//-/_}.wasm"
  if [ ! -f "$WASM_TARGET/$wasm_name" ]; then
    NEEDS_BUILD=true
    break
  fi
done

if [ "$NEEDS_BUILD" = true ]; then
  echo "==> Building WASM modules..."
  (cd "$MODULES_DIR" && cargo build --release --target wasm32-unknown-unknown)
fi

# Generate TypeScript bindings for each module
FAILED=()
for mod in "${MODULES[@]}"; do
  echo "==> Generating TypeScript bindings for $mod..."
  # Generate to hex-chat/ui
  OUT_DIR="$OUT_BASE/$mod"
  mkdir -p "$OUT_DIR"
  if spacetime generate \
    --lang typescript \
    --out-dir "$OUT_DIR" \
    --module-path "$MODULES_DIR/$mod" 2>&1; then
    echo "    OK: $mod (hex-chat)"
  else
    echo "    FAIL: $mod (hex-chat)"
    FAILED+=("$mod")
  fi

  # Generate to hex-nexus/assets (dashboard)
  DASH_DIR="$OUT_DASHBOARD/$mod"
  mkdir -p "$DASH_DIR"
  if spacetime generate \
    --lang typescript \
    --out-dir "$DASH_DIR" \
    --module-path "$MODULES_DIR/$mod" 2>&1; then
    echo "    OK: $mod (dashboard)"
  else
    echo "    FAIL: $mod (dashboard)"
  fi
done

if [ ${#FAILED[@]} -gt 0 ]; then
  echo ""
  echo "FAILED modules: ${FAILED[*]}"
  exit 1
fi

echo ""
echo "All TypeScript bindings generated successfully in $OUT_BASE/"

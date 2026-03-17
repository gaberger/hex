#!/usr/bin/env bash
# Build hex-core native module (Rust → NAPI-RS → Node addon)
# This is optional — hex works without it (falls back to WASM tree-sitter)
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"
HEX_CORE_DIR="$PROJECT_ROOT/hex-core"

# Check for Rust toolchain
if ! command -v cargo &>/dev/null; then
  echo "[hex-core] Rust toolchain not found — skipping native build"
  echo "[hex-core] Install Rust: https://rustup.rs/"
  exit 0
fi

# Check for napi-cli
if ! command -v napi &>/dev/null; then
  echo "[hex-core] Installing @napi-rs/cli..."
  npm install -g @napi-rs/cli 2>/dev/null || bun install -g @napi-rs/cli 2>/dev/null || {
    echo "[hex-core] Could not install @napi-rs/cli — skipping native build"
    exit 0
  }
fi

cd "$HEX_CORE_DIR"

echo "[hex-core] Building native tree-sitter module..."
napi build --release --platform

# Copy the built addon to node_modules so TypeScript can find it
ADDON_NAME="hex-core"
TARGET_DIR="$PROJECT_ROOT/node_modules/@hex/native"
mkdir -p "$TARGET_DIR"

# Copy .node file
cp -f "$HEX_CORE_DIR/$ADDON_NAME".*.node "$TARGET_DIR/" 2>/dev/null || \
cp -f "$HEX_CORE_DIR/index.node" "$TARGET_DIR/" 2>/dev/null || true

# Copy JS bindings and type definitions
cp -f "$HEX_CORE_DIR/index.js" "$TARGET_DIR/" 2>/dev/null || true
cp -f "$HEX_CORE_DIR/index.d.ts" "$TARGET_DIR/" 2>/dev/null || true
cp -f "$HEX_CORE_DIR/package.json" "$TARGET_DIR/"

echo "[hex-core] Native module installed to $TARGET_DIR"
echo "[hex-core] Build complete"

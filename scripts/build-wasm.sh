#!/usr/bin/env bash
# Rebuild all SpacetimeDB WASM modules and sync into hex-cli/assets/wasm/
# so the rust-embed bundle in hex-nexus/hex-cli ships with up-to-date bytes.
#
# Usage:
#   scripts/build-wasm.sh                # build all
#   scripts/build-wasm.sh hexflo-coordination agent-registry  # build specific
#
# Why: hex-nexus's launcher publishes WASM modules from the embedded bytes
# (see ADR-2604010000 + the embedded-wasm launcher fix). When operators
# edit spacetime-modules/<x>/src/lib.rs, the on-disk wasm in
# hex-cli/assets/wasm/<x>.wasm goes stale and the next nexus build will
# bake the OLD bytes. This script keeps them in sync.
#
# CI gate: scripts/check-wasm-fresh.sh fails when wasm assets are older
# than the corresponding source. Run this script to fix.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MODULES_DIR="$REPO_ROOT/spacetime-modules"
ASSETS_DIR="$REPO_ROOT/hex-cli/assets/wasm"
TARGET_DIR="$MODULES_DIR/target/wasm32-unknown-unknown/release"

ALL_MODULES=(hexflo-coordination agent-registry inference-gateway secret-grant chat-relay rl-engine neural-lab)

if [[ $# -gt 0 ]]; then
  MODULES=("$@")
else
  MODULES=("${ALL_MODULES[@]}")
fi

mkdir -p "$ASSETS_DIR"

# Verify the wasm32 target is installed before attempting builds — a clearer
# error than the cargo-internal one.
if ! rustup target list --installed 2>/dev/null | grep -q '^wasm32-unknown-unknown$'; then
  echo "ERROR: wasm32-unknown-unknown target not installed." >&2
  echo "  Install with: rustup target add wasm32-unknown-unknown" >&2
  exit 2
fi

cd "$MODULES_DIR"

OK=()
FAILED=()
for mod in "${MODULES[@]}"; do
  if [[ ! -d "$mod" ]]; then
    echo "WARN: module dir '$mod' not found — skipping" >&2
    continue
  fi
  echo ""
  echo "=== Building $mod ==="
  if cargo build -p "$mod" --target wasm32-unknown-unknown --release 2>&1 | tail -3; then
    wasm_name="${mod//-/_}.wasm"
    src="$TARGET_DIR/$wasm_name"
    dst="$ASSETS_DIR/$wasm_name"
    if [[ ! -f "$src" ]]; then
      echo "ERROR: build succeeded but $src not found" >&2
      FAILED+=("$mod")
      continue
    fi
    cp "$src" "$dst"
    bytes=$(stat -c%s "$dst" 2>/dev/null || stat -f%z "$dst")
    echo "  → $dst ($bytes bytes)"
    OK+=("$mod")
  else
    FAILED+=("$mod")
  fi
done

echo ""
echo "=== Summary ==="
echo "  built: ${#OK[@]} (${OK[*]:-})"
if (( ${#FAILED[@]} > 0 )); then
  echo "  FAILED: ${#FAILED[@]} (${FAILED[*]})"
  exit 1
fi
echo "  Run 'cargo build -p hex-nexus --release' to bake the new bytes into the binary."

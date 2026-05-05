#!/usr/bin/env bash
# CI gate: fail if any spacetime-module's source is newer than the embedded
# wasm in hex-cli/assets/wasm/. The launcher publishes from those embedded
# bytes; stale assets mean the deployed nexus will publish the OLD reducer
# behavior even though the source code shows new behavior.
#
# Exit codes:
#   0 — all wasm assets up-to-date
#   1 — at least one module is stale
#   2 — environment problem (missing dirs)
#
# Fix: scripts/build-wasm.sh [<module>...]

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MODULES_DIR="$REPO_ROOT/spacetime-modules"
ASSETS_DIR="$REPO_ROOT/hex-cli/assets/wasm"

if [[ ! -d "$MODULES_DIR" ]]; then
  echo "ERROR: $MODULES_DIR not found" >&2
  exit 2
fi
if [[ ! -d "$ASSETS_DIR" ]]; then
  echo "ERROR: $ASSETS_DIR not found" >&2
  exit 2
fi

# Newest mtime across all *.rs files in a directory tree.
newest_src_mtime() {
  local dir="$1"
  if [[ ! -d "$dir" ]]; then echo 0; return; fi
  find "$dir" -name '*.rs' -printf '%T@\n' 2>/dev/null \
    | sort -nr | head -1 | cut -d. -f1 || echo 0
}

ALL_MODULES=(hexflo-coordination agent-registry inference-gateway secret-grant chat-relay rl-engine neural-lab)

STALE=()
MISSING=()
for mod in "${ALL_MODULES[@]}"; do
  src_dir="$MODULES_DIR/$mod"
  wasm_file="$ASSETS_DIR/${mod//-/_}.wasm"
  if [[ ! -d "$src_dir" ]]; then
    continue
  fi
  if [[ ! -f "$wasm_file" ]]; then
    MISSING+=("$mod")
    continue
  fi
  src_mtime=$(newest_src_mtime "$src_dir")
  wasm_mtime=$(stat -c%Y "$wasm_file" 2>/dev/null || stat -f%m "$wasm_file")
  if (( src_mtime > wasm_mtime )); then
    src_age=$((src_mtime - wasm_mtime))
    STALE+=("$mod (src ${src_age}s newer than wasm)")
  fi
done

if (( ${#MISSING[@]} == 0 && ${#STALE[@]} == 0 )); then
  echo "✓ all 7 wasm assets up-to-date"
  exit 0
fi

echo "✗ wasm assets out of sync with source"
if (( ${#MISSING[@]} > 0 )); then
  echo "  missing: ${MISSING[*]}"
fi
if (( ${#STALE[@]} > 0 )); then
  echo "  stale:"
  printf '    - %s\n' "${STALE[@]}"
fi
echo ""
echo "Fix: $REPO_ROOT/scripts/build-wasm.sh ${MISSING[*]:-} ${STALE[*]%% *}"
exit 1

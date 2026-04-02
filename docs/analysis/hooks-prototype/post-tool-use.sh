#!/usr/bin/env bash
# post-tool-use.sh — PostToolUse hook for hex architecture enforcement
#
# Claude Code passes the tool result as JSON on stdin after every tool use.
# We must exit 0 and print JSON: {} (no action) or {"additionalContext": "..."}
#
# Registered for: Edit, Write tools.

set -euo pipefail

INPUT="$(cat)"

no_action() {
  printf '{}'
  exit 0
}

warn() {
  # Escape the message for JSON
  local msg
  msg="$(printf '%s' "$1" | python3 -c "import sys,json; print(json.dumps(sys.stdin.read()))" 2>/dev/null || printf '"%s"' "$1")"
  printf '{"additionalContext":%s}' "$msg"
  exit 0
}

# ── Identify tool ────────────────────────────────────────────────────────────
TOOL_NAME="$(printf '%s' "$INPUT" | python3 -c "import sys,json; d=json.load(sys.stdin); print(d.get('tool_name',''))" 2>/dev/null || echo "")"

if [ "$TOOL_NAME" != "Edit" ] && [ "$TOOL_NAME" != "Write" ]; then
  no_action
fi

# ── Extract the file that was just written ────────────────────────────────────
FILE_PATH="$(printf '%s' "$INPUT" | python3 -c "
import sys, json
d = json.load(sys.stdin)
inp = d.get('tool_input', {})
print(inp.get('file_path', inp.get('path', '')))
" 2>/dev/null || echo "")"

if [ -z "$FILE_PATH" ] || [ ! -f "$FILE_PATH" ]; then
  no_action
fi

# ── Classify the layer of the written file ────────────────────────────────────
classify_layer() {
  local p="$1"
  case "$p" in
    */core/domain/*|*/domain/*)      echo "domain" ;;
    */core/ports/*|*/ports/*)        echo "ports" ;;
    */core/usecases/*|*/usecases/*)  echo "usecases" ;;
    */adapters/primary/*)            echo "adapter_primary" ;;
    */adapters/secondary/*)          echo "adapter_secondary" ;;
    */composition-root*)             echo "composition_root" ;;
    *)                               echo "other" ;;
  esac
}

TARGET_LAYER="$(classify_layer "$FILE_PATH")"
if [ "$TARGET_LAYER" = "other" ] || [ "$TARGET_LAYER" = "composition_root" ]; then
  no_action
fi

# ── Scan the file for boundary violations ─────────────────────────────────────
VIOLATIONS=()

# Read actual import lines from the written file
IMPORT_LINES="$(grep -nE "^(import|from|const .* require|use )" "$FILE_PATH" 2>/dev/null || true)"

scan_imports() {
  local layer="$1"
  while IFS= read -r line; do
    [ -z "$line" ] && continue

    # Extract the module path from TS/JS/Rust imports
    imp="$(printf '%s' "$line" | grep -oE "from ['\"][^'\"]+['\"]|require\(['\"][^'\"]+['\"]\)" | grep -oE "['\"][^'\"]+['\"]" | tr -d "'\"" | head -1 || true)"
    # Also catch Rust: use foo::bar
    if [ -z "$imp" ]; then
      imp="$(printf '%s' "$line" | grep -oE "^use [a-z_::]+" | sed 's/^use //' || true)"
    fi
    [ -z "$imp" ] && continue

    linenum="$(printf '%s' "$line" | cut -d: -f1)"

    case "$layer" in
      domain)
        if [[ "$imp" == *"/ports/"* ]] || [[ "$imp" == *"/adapters/"* ]]; then
          VIOLATIONS+=("Line $linenum: domain must not import '$imp'")
        fi
        ;;
      ports)
        if [[ "$imp" == *"/adapters/"* ]]; then
          VIOLATIONS+=("Line $linenum: ports must not import adapters '$imp'")
        fi
        ;;
      usecases)
        if [[ "$imp" == *"/adapters/"* ]]; then
          VIOLATIONS+=("Line $linenum: usecases must not import adapters '$imp'")
        fi
        ;;
      adapter_primary|adapter_secondary)
        if [[ "$imp" == *"/adapters/"* ]]; then
          VIOLATIONS+=("Line $linenum: adapter must not import another adapter '$imp'")
        fi
        if [[ "$imp" == *"/core/domain/"* ]] || [[ "$imp" == *"../domain/"* ]]; then
          VIOLATIONS+=("Line $linenum: adapter must import via ports/, not domain/ directly: '$imp'")
        fi
        ;;
    esac
  done <<< "$IMPORT_LINES"
}

scan_imports "$TARGET_LAYER"

if [ ${#VIOLATIONS[@]} -eq 0 ]; then
  no_action
fi

# Build warning message
VIOLATION_TEXT="HEX ARCHITECTURE VIOLATION in ${FILE_PATH} (layer: ${TARGET_LAYER}):\\n"
for v in "${VIOLATIONS[@]}"; do
  VIOLATION_TEXT+="  - ${v}\\n"
done
VIOLATION_TEXT+="\\nRun \`hex analyze .\` to see the full report. Fix these before committing."

warn "$VIOLATION_TEXT"

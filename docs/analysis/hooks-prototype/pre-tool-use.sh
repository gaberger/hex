#!/usr/bin/env bash
# pre-tool-use.sh — PreToolUse hook for hex architecture enforcement
#
# Claude Code passes the tool input as JSON on stdin.
# We must exit 0 and print JSON: {"permissionDecision": "allow"}
#                              or {"permissionDecision": "deny", "permissionDecisionReason": "..."}
#
# Hook is registered for: Bash, Edit, Write tools.

set -euo pipefail

INPUT="$(cat)"

deny() {
  printf '{"permissionDecision":"deny","permissionDecisionReason":"%s"}' "$1"
  exit 0
}

allow() {
  printf '{"permissionDecision":"allow"}'
  exit 0
}

# ── Identify tool ────────────────────────────────────────────────────────────
TOOL_NAME="$(printf '%s' "$INPUT" | python3 -c "import sys,json; d=json.load(sys.stdin); print(d.get('tool_name',''))" 2>/dev/null || echo "")"

# ── Bash: block dangerous patterns ───────────────────────────────────────────
if [ "$TOOL_NAME" = "Bash" ]; then
  COMMAND="$(printf '%s' "$INPUT" | python3 -c "import sys,json; d=json.load(sys.stdin); print(d.get('tool_input',{}).get('command',''))" 2>/dev/null || echo "")"

  DANGEROUS_PATTERNS=(
    'rm[[:space:]]+-rf[[:space:]]+/'
    'rm[[:space:]]+-rf[[:space:]]+\*'
    'dd[[:space:]]+if='
    'mkfs\.'
    ':[[:space:]]*\(\)[[:space:]]*\{.*fork'   # fork bomb
    'chmod[[:space:]]+-R[[:space:]]+777'
    '> /dev/sd'
    'shred '
    'wipefs '
  )

  for pattern in "${DANGEROUS_PATTERNS[@]}"; do
    if printf '%s' "$COMMAND" | grep -qE "$pattern" 2>/dev/null; then
      deny "Blocked dangerous bash pattern: $pattern"
    fi
  done

  allow
fi

# ── Edit / Write: check hex layer boundary violations ────────────────────────
if [ "$TOOL_NAME" = "Edit" ] || [ "$TOOL_NAME" = "Write" ]; then
  FILE_PATH="$(printf '%s' "$INPUT" | python3 -c "import sys,json; d=json.load(sys.stdin); inp=d.get('tool_input',{}); print(inp.get('file_path', inp.get('path','')))" 2>/dev/null || echo "")"
  NEW_CONTENT="$(printf '%s' "$INPUT" | python3 -c "import sys,json; d=json.load(sys.stdin); inp=d.get('tool_input',{}); print(inp.get('new_string', inp.get('content','')))" 2>/dev/null || echo "")"

  if [ -z "$FILE_PATH" ]; then
    allow
  fi

  # Determine which hex layer the target file lives in
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

  if [ "$TARGET_LAYER" = "other" ]; then
    allow
  fi

  # Check new content for forbidden import patterns
  # We scan for TypeScript/Rust import lines and validate against layer rules.

  check_ts_imports() {
    local layer="$1"
    local content="$2"
    # Extract import paths from TS/JS: import ... from '...' or require('...')
    local import_paths
    import_paths="$(printf '%s' "$content" | grep -oE "from ['\"][^'\"]+['\"]|require\(['\"][^'\"]+['\"]\)" | grep -oE "['\"][^'\"]+['\"]" | tr -d "'\"" || true)"

    while IFS= read -r imp; do
      [ -z "$imp" ] && continue
      imp_layer="$(classify_layer "$imp")"

      case "$layer" in
        domain)
          # domain may not import from ports/, adapters/
          if [[ "$imp" == *"/ports/"* ]] || [[ "$imp" == *"/adapters/"* ]]; then
            echo "domain layer must not import from: $imp"
            return
          fi
          ;;
        ports)
          # ports may not import from adapters/
          if [[ "$imp" == *"/adapters/"* ]]; then
            echo "ports layer must not import from adapters: $imp"
            return
          fi
          ;;
        usecases)
          # usecases may not import from adapters/
          if [[ "$imp" == *"/adapters/"* ]]; then
            echo "usecases layer must not import from adapters: $imp"
            return
          fi
          ;;
        adapter_primary|adapter_secondary)
          # adapters must not import other adapters
          if [[ "$imp" == *"/adapters/"* ]]; then
            echo "adapters must not import other adapters: $imp"
            return
          fi
          # adapters must not import domain directly (must go via ports)
          if [[ "$imp" == *"/core/domain/"* ]] || [[ "$imp" == *"/domain/"* ]]; then
            echo "adapters must import via ports/, not domain/ directly: $imp"
            return
          fi
          ;;
      esac
    done <<< "$import_paths"
    echo ""
  }

  VIOLATION="$(check_ts_imports "$TARGET_LAYER" "$NEW_CONTENT")"
  if [ -n "$VIOLATION" ]; then
    SAFE_VIOLATION="${VIOLATION//\"/\\\"}"
    deny "Hex boundary violation in $TARGET_LAYER: $SAFE_VIOLATION"
  fi

  allow
fi

# Default: allow anything else
allow

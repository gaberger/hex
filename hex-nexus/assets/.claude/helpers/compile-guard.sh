#!/usr/bin/env bash
# compile-guard.sh — Prevent concurrent cargo build/check/test/clippy across Claude sessions.
#
# Usage (from Claude Code hooks):
#   bash hex-cli/assets/helpers/compile-guard.sh --pre   # PreToolUse/Bash
#   bash hex-cli/assets/helpers/compile-guard.sh --post  # PostToolUse/Bash
#
# Lock file: ~/.hex/compile.lock  (PID:session_id:timestamp)
# Exit 2 from --pre blocks the Bash tool call and shows the message to Claude.

set -euo pipefail

MODE="${1:---pre}"
LOCK_FILE="${HOME}/.hex/compile.lock"
SESSION_ID="${CLAUDE_SESSION_ID:-unknown}"

# Read tool input from stdin (Claude Code hook JSON format)
INPUT="$(cat)"
CMD="$(printf '%s' "$INPUT" | python3 -c "
import sys, json
try:
    d = json.load(sys.stdin)
    print(d.get('tool_input', {}).get('command', ''))
except Exception:
    print('')
" 2>/dev/null || true)"

# Only guard cargo compile commands
if ! printf '%s' "$CMD" | grep -qE 'cargo[[:space:]]+(build|check|test|clippy|nexus)'; then
    exit 0
fi

mkdir -p "${HOME}/.hex"

case "$MODE" in
  --pre)
    if [ -f "$LOCK_FILE" ]; then
        LOCK_PID="$(cut -d: -f1 "$LOCK_FILE" 2>/dev/null || echo '')"
        LOCK_SESSION="$(cut -d: -f2 "$LOCK_FILE" 2>/dev/null || echo '')"
        LOCK_TIME="$(cut -d: -f3 "$LOCK_FILE" 2>/dev/null || echo '')"

        # Is this our own lock (e.g. re-entrant)? Allow it.
        if [ "$LOCK_SESSION" = "$SESSION_ID" ]; then
            exit 0
        fi

        # Is the locking process still alive?
        if [ -n "$LOCK_PID" ] && kill -0 "$LOCK_PID" 2>/dev/null; then
            echo "BLOCKED: cargo compile in progress"
            echo "  Session : $LOCK_SESSION"
            echo "  PID     : $LOCK_PID"
            echo "  Since   : $LOCK_TIME"
            echo ""
            echo "Wait for the other session to finish, or clear with:"
            echo "  rm ${LOCK_FILE}"
            exit 2
        fi

        # Stale lock — process is dead, clean up and proceed
        rm -f "$LOCK_FILE"
    fi

    # Acquire lock
    printf '%s:%s:%s\n' "$$" "$SESSION_ID" "$(date -u +%Y-%m-%dT%H:%M:%SZ)" > "$LOCK_FILE"
    exit 0
    ;;

  --post)
    if [ -f "$LOCK_FILE" ]; then
        LOCK_SESSION="$(cut -d: -f2 "$LOCK_FILE" 2>/dev/null || echo '')"
        if [ "$LOCK_SESSION" = "$SESSION_ID" ]; then
            rm -f "$LOCK_FILE"
        fi
    fi
    exit 0
    ;;

  --status)
    if [ ! -f "$LOCK_FILE" ]; then
        echo "compile-guard: no active lock"
        exit 0
    fi
    LOCK_PID="$(cut -d: -f1 "$LOCK_FILE")"
    LOCK_SESSION="$(cut -d: -f2 "$LOCK_FILE")"
    LOCK_TIME="$(cut -d: -f3 "$LOCK_FILE")"
    if kill -0 "$LOCK_PID" 2>/dev/null; then
        echo "compile-guard: LOCKED by session $LOCK_SESSION (pid $LOCK_PID) since $LOCK_TIME"
    else
        echo "compile-guard: stale lock (pid $LOCK_PID dead) — safe to remove: rm $LOCK_FILE"
    fi
    exit 0
    ;;

  --clear)
    rm -f "$LOCK_FILE"
    echo "compile-guard: lock cleared"
    exit 0
    ;;
esac

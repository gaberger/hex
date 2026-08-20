#!/usr/bin/env bash
# Per the SpacetimeDB-only directive: SQLite is fully removed from the hex
# Rust workspace. This gate keeps it out. New code that adds rusqlite/sqlx
# deps or claims dual-backend behavior in doc-strings/comments fails CI.
#
# Allowed exceptions are listed below — these are deliberate references that
# are not re-introducing SQLite (historical notes, policy statements, generic
# user-prompt keywords, test fixtures).

set -euo pipefail

ROOT="${1:-$(cd "$(dirname "$0")/.." && pwd)}"

# Hard-fail set: any of these strings in a Rust file or Cargo.toml is a real
# re-introduction.
HARD_FAIL_PATTERNS=(
  'rusqlite'
  'sqlx'
  'hub\.db'
  '\.hex/hub'
  'SqliteStateAdapter'
  'SqliteEventAdapter'
  'SqliteSessionAdapter'
)

# Allow-list: file:line snippets we intentionally keep. Anything matching
# `(SQLite|sqlite)` outside HARD_FAIL_PATTERNS is checked against this list.
# Keep entries minimal — every entry is a debt to revisit.
ALLOWED=(
  'hex-cli/src/commands/plan/mod.rs:.*lower.contains."sqlite"'                      # generic user-prompt classifier
  'hex-nexus/src/adapters/events.rs:.*Replaces the former SQLite-backed'            # historical note
  'hex-nexus/src/state_config.rs:.*SQLite has been removed'                         # policy statement
  'hex-nexus/src/adapters/spacetime_session.rs:.*no SQLite fallback'                # policy statement
  'hex-nexus/src/orchestration/regression.rs:.*adapters/secondary/sqlite.rs'        # test fixture (generic filename)
  'hex-nexus/src/ports/state.rs:.*SQLite was removed per the STDB-only directive'   # policy explanation
)

fail=0

# 1. Hard-fail patterns — never allowed.
for pat in "${HARD_FAIL_PATTERNS[@]}"; do
  hits=$(grep -rInE "$pat" --include='*.rs' --include='Cargo.toml' \
    "$ROOT/hex-cli" "$ROOT/hex-nexus" "$ROOT/hex-core" "$ROOT/hex-agent" 2>/dev/null || true)
  if [ -n "$hits" ]; then
    echo "FAIL: hard-banned pattern '$pat' found:"
    echo "$hits"
    fail=1
  fi
done

# 2. SQLite mentions in Rust source — must be on the allow-list.
all_sqlite=$(grep -rInE 'SQLite|sqlite' --include='*.rs' \
  "$ROOT/hex-cli" "$ROOT/hex-nexus" "$ROOT/hex-core" "$ROOT/hex-agent" 2>/dev/null || true)

while IFS= read -r line; do
  [ -z "$line" ] && continue
  rel="${line#$ROOT/}"
  matched=0
  for allow in "${ALLOWED[@]}"; do
    if echo "$rel" | grep -qE "$allow"; then
      matched=1
      break
    fi
  done
  if [ "$matched" -eq 0 ]; then
    echo "FAIL: unallowed SQLite reference (add to ALLOWED if intentional): $rel"
    fail=1
  fi
done <<< "$all_sqlite"

if [ "$fail" -eq 0 ]; then
  echo "OK: no SQLite re-introduction detected."
fi

exit "$fail"

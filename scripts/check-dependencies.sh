#!/usr/bin/env bash
# check-dependencies.sh — Hexagonal architecture dependency-direction gate.
#
# Enforces the hex import rules (CLAUDE.md, ADR-001):
#   1. domain/    imports only domain/
#   2. ports/     imports domain/ only
#   3. usecases/  imports domain/ + ports/ only
#   4. adapters/  import ports/ only (never another adapter)
#   5. composition* is the ONLY file allowed to import from adapters/
#
# Two layered checks, in order:
#   A. `hex analyze .` — canonical tree-sitter analyzer with grade + boundary
#      violation count. Authoritative source of truth.
#   B. Fast grep-based redundant check focused on the single rule most prone
#      to silent regression: cross-adapter coupling (rule 4) outside of
#      `#[cfg(test)]` modules. Catches obvious diffs even before the hex
#      binary is built. Test modules are exempt — tests legitimately import
#      sibling adapters as fakes/fixtures.
#
# Exit 0 = clean. Exit 1 = at least one boundary violation reported by either
# Phase A or Phase B.
#
# Workplan: wp-hexagonal-architecture-foundation P1.3
# ADR: ADR-001

set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

violations=0
report() {
    echo "::error::$1" >&2
    violations=$((violations + 1))
}

# ----------------------------------------------------------------------------
# Phase A — canonical analyzer.
# ----------------------------------------------------------------------------
echo "[check-dependencies] Phase A — hex analyze (canonical)"

HEX_BIN=""
if command -v hex >/dev/null 2>&1; then
    HEX_BIN="hex"
elif [ -x "./target/release/hex" ]; then
    HEX_BIN="./target/release/hex"
elif [ -x "./target/release/hex-cli" ]; then
    HEX_BIN="./target/release/hex-cli"
fi

if [ -n "$HEX_BIN" ]; then
    # Capture output so we can inspect boundary-violation count even on exit 0.
    if ! out=$("$HEX_BIN" analyze . 2>&1); then
        echo "$out"
        report "hex analyze . exited non-zero"
    else
        echo "$out"
        # Defensive: if analyzer prints any non-zero violation count, fail.
        if echo "$out" | grep -qE '[1-9][0-9]* boundary violations'; then
            report "hex analyze reported boundary violations (see output above)"
        fi
    fi
else
    echo "[check-dependencies] hex binary not found — skipping Phase A"
    echo "[check-dependencies] (build with: cargo build -p hex-cli --release)"
fi

echo

# ----------------------------------------------------------------------------
# Phase B — cross-adapter coupling grep (rule 4), test-module aware.
# Runs even if hex binary is missing, so the gate has *some* signal in a
# bare-checkout CI step. Awk tracks `#[cfg(test)]` regions so test-only
# imports aren't flagged.
# ----------------------------------------------------------------------------
echo "[check-dependencies] Phase B — cross-adapter coupling scan"

SCAN_ROOTS=(
    "hex-core/src"
    "hex-nexus/src"
    "src"
)

# Files where adapter→adapter imports are allowed (composition / wiring).
is_exempt_file() {
    case "$1" in
        */composition_root.rs|*/composition.rs|*/composition/*) return 0 ;;
        */lib.rs|*/main.rs)                                     return 0 ;;
        */build.rs)                                             return 0 ;;
        *) return 1 ;;
    esac
}

scanned=0
for root in "${SCAN_ROOTS[@]}"; do
    [ -d "$root" ] || continue
    while IFS= read -r -d '' file; do
        # Only enforce on files inside an adapters/ tree.
        case "$file" in
            */adapters/*) ;;
            *) continue ;;
        esac

        is_exempt_file "$file" && continue
        scanned=$((scanned + 1))

        # Awk skips lines inside #[cfg(test)] mod blocks. Brace depth tracking
        # is good enough for Rust's standard test-module convention:
        #   #[cfg(test)]
        #   mod tests { ... }
        offending=$(awk '
            BEGIN { in_test = 0; depth = 0; pending_cfg_test = 0 }
            /#\[cfg\(test\)\]/             { pending_cfg_test = 1; next }
            /#\[cfg\(any\(.*test.*\)\)\]/  { pending_cfg_test = 1; next }
            {
                # Count braces on this line via split() — portable across
                # awk variants (mawk/nawk/gawk).
                n_open  = split($0, _o, "{") - 1
                n_close = split($0, _c, "}") - 1
                if (pending_cfg_test && n_open > 0) {
                    in_test = 1
                    depth = n_open - n_close
                    pending_cfg_test = 0
                    next
                }
                if (in_test) {
                    depth += n_open - n_close
                    if (depth <= 0) { in_test = 0; depth = 0 }
                    next
                }
                if (/^[[:space:]]*use[[:space:]]+(crate|super)::adapters::/) {
                    print NR ": " $0
                }
            }
        ' "$file")

        if [ -n "$offending" ]; then
            while IFS= read -r line; do
                report "$file: $line — cross-adapter import (rule 4)"
            done <<< "$offending"
        fi
    done < <(find "$root" -name '*.rs' -type f -print0)
done

echo "[check-dependencies] Phase B scanned $scanned adapter file(s)"
echo

# ----------------------------------------------------------------------------
# Verdict
# ----------------------------------------------------------------------------
if [ $violations -eq 0 ]; then
    echo "[check-dependencies] OK — no dependency-direction violations"
    exit 0
fi

echo "[check-dependencies] FAIL — $violations violation(s) found"
exit 1

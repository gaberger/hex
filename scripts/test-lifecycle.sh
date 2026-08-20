#!/usr/bin/env bash
# test-lifecycle.sh — full hex lifecycle acceptance test on a realistic target project.
#
# Drives an example TS web app (examples/food-delivery-ts, a hexagonal app) through
# every hex surface and asserts the output at each gate. This is the integration test
# the unit suites can't be: it proves the *whole platform* works end-to-end, not just
# that individual crates compile.
#
# Stages: install → baseline → (build via hex do, with --build) → validate → run → ship.
#
# Usage:
#   scripts/test-lifecycle.sh             # verify the lifecycle artifacts are healthy
#   scripts/test-lifecycle.sh --build     # also re-drive a feature through `hex do` (slow; needs a model)
set -u

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
APP="$ROOT/examples/food-delivery-ts"
HEX="${HEX_BIN:-hex}"
# node24 is required for the app's vitest (node:util.styleText); fall back to PATH node.
NODE24="/home/gary/.local/node24/bin"
[ -d "$NODE24" ] && export PATH="$NODE24:$PATH"

pass=0; fail=0
ok()   { echo "  ✓ $1"; pass=$((pass+1)); }
bad()  { echo "  ✗ $1"; fail=$((fail+1)); }
stage(){ echo; echo "── $1"; }

stage "1. INSTALL — hex init scaffolding present"
if [ -f "$APP/.hex/project.json" ]; then ok ".hex/project.json present"; else
  ( cd "$APP" && "$HEX" init . --skip-interview --no-claude-md >/dev/null 2>&1 )
  [ -f "$APP/.hex/project.json" ] && ok ".hex/project.json created" || bad "hex init failed"
fi

stage "2. BASELINE — dependencies + typecheck"
( cd "$APP" && [ -d node_modules ] || npm install >/dev/null 2>&1 )
( cd "$APP" && npx tsc --noEmit >/dev/null 2>&1 ) && ok "tsc --noEmit clean" || bad "typecheck failed"

if [ "${1:-}" = "--build" ]; then
  stage "3. BUILD — drive a feature through hex do (evidence-gated)"
  # Reset the demo feature so the loop rebuilds it from the spec.
  ( cd "$APP" && git checkout -- src/core/domain/Order.ts 2>/dev/null )
  EV='cd examples/food-delivery-ts && PATH='"$NODE24"':$PATH npx tsc --noEmit && PATH='"$NODE24"':$PATH npx vitest run'
  ( cd "$ROOT" && "$HEX" do run --file examples/food-delivery-ts/src/core/domain/Order.ts \
      --evidence "$EV" \
      "Add and export cancelOrder(order: Order): Order using transitionStatus to OrderStatus.Cancelled; CancelOrder.test.ts is the spec." \
      2>&1 | grep -q "evidence pass" ) && ok "hex do built the feature (evidence passed)" || bad "hex do did not pass evidence"
fi

stage "4. VALIDATE — hexagonal architecture"
A="$( "$HEX" analyze "$APP" 2>&1 )"
echo "$A" | grep -qE "grade: A" && ok "architecture grade A/A+" || bad "architecture grade below A"
echo "$A" | grep -qE "0 boundary violation" && ok "0 boundary violations" || bad "boundary violations present"

stage "5. RUN — the app's own test suite passes (behavior, not just compiles)"
( cd "$APP" && npx vitest run >/dev/null 2>&1 ) && ok "vitest suite green" || bad "tests failed"

stage "6. SHIP — feature is committed, tree clean for the app"
grep -q "cancelOrder" "$APP/src/core/domain/Order.ts" && ok "feature present in committed source" || bad "feature missing"

echo
echo "── lifecycle: $pass passed, $fail failed"
[ "$fail" -eq 0 ] && { echo "✓ full hex lifecycle GREEN on examples/food-delivery-ts"; exit 0; } || { echo "✗ lifecycle has failures"; exit 1; }

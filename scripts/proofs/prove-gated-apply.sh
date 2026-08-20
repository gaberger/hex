#!/usr/bin/env bash
# Proof harness for ADR-2026-05-23-0900 Path B item 4:
# persona_prompt_apply_gated enforces verdict + provider-divergence
# fail-closed at the STDB reducer boundary.
#
# Tests every gate condition individually, asserts:
#   - rejection scenarios return HTTP 530 AND leave state unchanged
#   - acceptance scenario returns HTTP 200 AND appends history + proposal
#   - state is rolled back so test runs are idempotent
#
# Run while a hex-nexus + STDB stack is up. Exit 0 = all gates held.
# Exit 1 = at least one assertion failed.

set -uo pipefail

STDB=http://127.0.0.1:3033
DB=hex
ROLE=cto

PASS=0
FAIL=0

bold() { printf '\033[1m%s\033[0m\n' "$1"; }
green() { printf '\033[32m%s\033[0m\n' "$1"; }
red() { printf '\033[31m%s\033[0m\n' "$1"; }

# Snapshot the current state of `persona_prompt`, history-count, proposal-count.
snapshot() {
  curl -sS -X POST "$STDB/v1/database/$DB/sql" \
    -H "Content-Type: text/plain" \
    --data "SELECT classify_body, seeded_by FROM persona_prompt WHERE role='$ROLE'" \
    | python3 -c "import json,sys;d=json.load(sys.stdin);r=d[0]['rows'][0];print(r[0][:40]); print(r[1])"
}

history_count() {
  curl -sS -X POST "$STDB/v1/database/$DB/sql" \
    -H "Content-Type: text/plain" \
    --data "SELECT version FROM persona_prompt_history WHERE role='$ROLE'" \
    | python3 -c "import json,sys; print(len(json.load(sys.stdin)[0]['rows']))"
}

proposal_count() {
  curl -sS -X POST "$STDB/v1/database/$DB/sql" \
    -H "Content-Type: text/plain" \
    --data "SELECT id FROM persona_prompt_proposal WHERE role='$ROLE'" \
    | python3 -c "import json,sys; print(len(json.load(sys.stdin)[0]['rows']))"
}

# call_gated <role> <classify> <reason> <model_pref> <model_upg> <red_prov> <red_v> <blue_prov> <blue_v> <judge_v>
# Echoes "<http_code>|<body>"
call_gated() {
  local payload
  payload=$(python3 -c "import json,sys; print(json.dumps(sys.argv[1:]))" "$@")
  local resp http
  resp=$(curl -sS -o /tmp/gate-body -w "%{http_code}" -X POST \
    "$STDB/v1/database/$DB/call/persona_prompt_apply_gated" \
    -H "Content-Type: application/json" \
    -d "$payload")
  printf '%s|%s' "$resp" "$(cat /tmp/gate-body)"
}

# assert_reject <test-name> <expected-substring> -- <args to call_gated>
assert_reject() {
  local name="$1" needle="$2"; shift 2
  local before_body before_hist before_prop
  before_body=$(snapshot)
  before_hist=$(history_count)
  before_prop=$(proposal_count)

  local result http body
  result=$(call_gated "$@")
  http="${result%%|*}"
  body="${result#*|}"

  local after_body after_hist after_prop
  after_body=$(snapshot)
  after_hist=$(history_count)
  after_prop=$(proposal_count)

  local ok=true
  [[ "$http" == "530" ]] || ok=false
  [[ "$body" == *"$needle"* ]] || ok=false
  [[ "$before_body" == "$after_body" ]] || ok=false
  [[ "$before_hist" == "$after_hist" ]] || ok=false
  [[ "$before_prop" == "$after_prop" ]] || ok=false

  if $ok; then
    green "PASS  $name"
    echo   "      HTTP=$http  msg='$body'  state-unchanged=yes"
    PASS=$((PASS+1))
  else
    red   "FAIL  $name"
    echo   "      HTTP=$http  msg='$body'"
    echo   "      state before: hist=$before_hist prop=$before_prop body=$before_body"
    echo   "      state after : hist=$after_hist prop=$after_prop body=$after_body"
    echo   "      needle='$needle'"
    FAIL=$((FAIL+1))
  fi
}

# assert_apply: positive case — must return 200 AND increment history + proposal AND change body
assert_apply() {
  local name="$1"; shift
  local before_body before_hist before_prop
  before_body=$(snapshot)
  before_hist=$(history_count)
  before_prop=$(proposal_count)

  local result http body
  result=$(call_gated "$@")
  http="${result%%|*}"
  body="${result#*|}"

  local after_body after_hist after_prop
  after_body=$(snapshot)
  after_hist=$(history_count)
  after_prop=$(proposal_count)

  local ok=true
  [[ "$http" == "200" ]] || ok=false
  [[ "$before_body" != "$after_body" ]] || ok=false
  [[ "$after_hist" -gt "$before_hist" ]] || ok=false
  [[ "$after_prop" -gt "$before_prop" ]] || ok=false

  if $ok; then
    green "PASS  $name"
    echo   "      HTTP=$http  hist:$before_hist→$after_hist  prop:$before_prop→$after_prop  body=changed"
    PASS=$((PASS+1))
  else
    red   "FAIL  $name"
    echo   "      HTTP=$http  body='$body'"
    echo   "      state before: hist=$before_hist prop=$before_prop body=$before_body"
    echo   "      state after : hist=$after_hist prop=$after_prop body=$after_body"
    FAIL=$((FAIL+1))
  fi
}

bold "ADR-2026-05-23-0900 Item-4 gated-apply proof"
echo "STDB=$STDB  DB=$DB  ROLE=$ROLE"
echo
bold "── REJECTION MATRIX ──────────────────────────────────────"

assert_reject "G1: empty role" \
  "role is required" \
  "" "b" "b" "m1" "m2" "anthropic" "approve" "ollama" "approve" "approve"

# Build an oversized body (8193 bytes — one over PERSONA_PROMPT_BODY_MAX)
OVERSIZE=$(python3 -c "print('x'*8193)")
assert_reject "G2: classify_body over 8192 bytes" \
  "exceeds 8192 byte cap" \
  "$ROLE" "$OVERSIZE" "ok" "m1" "m2" "anthropic" "approve" "ollama" "approve" "approve"

assert_reject "G3: reason_body over 8192 bytes" \
  "exceeds 8192 byte cap" \
  "$ROLE" "ok" "$OVERSIZE" "m1" "m2" "anthropic" "approve" "ollama" "approve" "approve"

assert_reject "G4: red verdict = reject" \
  "red verdict 'reject' is not approving" \
  "$ROLE" "body" "body" "m1" "m2" "anthropic" "reject" "ollama" "approve" "approve"

assert_reject "G5: blue verdict = reject" \
  "blue verdict 'reject' is not approving" \
  "$ROLE" "body" "body" "m1" "m2" "anthropic" "approve" "ollama" "reject" "approve"

assert_reject "G6: judge verdict = reject" \
  "judge verdict 'reject' is not approving" \
  "$ROLE" "body" "body" "m1" "m2" "anthropic" "approve" "ollama" "approve" "reject"

assert_reject "G7: red_provider empty" \
  "must both be specified" \
  "$ROLE" "body" "body" "m1" "m2" "" "approve" "ollama" "approve" "approve"

assert_reject "G8: blue_provider empty" \
  "must both be specified" \
  "$ROLE" "body" "body" "m1" "m2" "anthropic" "approve" "" "approve" "approve"

assert_reject "G9: red_provider == blue_provider (collision)" \
  "provider divergence violation" \
  "$ROLE" "body" "body" "m1" "m2" "anthropic" "approve" "anthropic" "approve" "approve"

assert_reject "G10: unseeded role" \
  "not yet seeded" \
  "not-a-real-role" "body" "body" "m1" "m2" "anthropic" "approve" "ollama" "approve" "approve"

echo
bold "── BYPASS NOISE — bad verdict strings the model might produce ──"

assert_reject "G11: red verdict = 'maybe'" \
  "red verdict 'maybe' is not approving" \
  "$ROLE" "body" "body" "m1" "m2" "anthropic" "maybe" "ollama" "approve" "approve"

assert_reject "G12: red verdict = uppercase APPROVE" \
  "red verdict 'APPROVE' is not approving" \
  "$ROLE" "body" "body" "m1" "m2" "anthropic" "APPROVE" "ollama" "approve" "approve"

assert_reject "G13: red verdict = empty string" \
  "red verdict '' is not approving" \
  "$ROLE" "body" "body" "m1" "m2" "anthropic" "" "ollama" "approve" "approve"

echo
bold "── ACCEPTANCE — all gates passed ─────────────────────────"

UNIQ_BODY="PROOF_HARNESS_$(date +%s)_$(uuidgen 2>/dev/null | head -c 8 || echo $$)"
assert_apply "G14: valid + approve + diverged providers" \
  "$ROLE" "$UNIQ_BODY" "$UNIQ_BODY" "qwen2.5-coder:14b" "claude-sonnet-4-6" \
  "anthropic" "approve" "ollama" "approve-with-changes" "approve"

echo
bold "── PROPOSAL LEDGER ASSERTION ─────────────────────────────"
LATEST=$(curl -sS -X POST "$STDB/v1/database/$DB/sql" \
  -H "Content-Type: text/plain" \
  --data "SELECT role, decision, red_provider, blue_provider FROM persona_prompt_proposal WHERE role='$ROLE'" \
  | python3 -c "import json,sys; rows=json.load(sys.stdin)[0]['rows']; print(rows[-1] if rows else '<empty>')")
echo "  Latest proposal row: $LATEST"
if [[ "$LATEST" == *"applied"* && "$LATEST" == *"anthropic"* && "$LATEST" == *"ollama"* ]]; then
  green "PASS  proposal ledger recorded the approved write with divergent providers"
  PASS=$((PASS+1))
else
  red   "FAIL  proposal ledger row missing or malformed"
  FAIL=$((FAIL+1))
fi

echo
bold "── CLEANUP — roll back the proof's apply ─────────────────"
LAST_GOOD=9
curl -sS -X POST "$STDB/v1/database/$DB/call/persona_prompt_rollback" \
  -H "Content-Type: application/json" -d "[\"$ROLE\", $LAST_GOOD]" \
  -w "  rollback HTTP=%{http_code}\n" >/dev/null
FINAL=$(snapshot | head -1)
if [[ "$FINAL" != *"PROOF_HARNESS"* ]]; then
  green "PASS  rolled back to clean body"
  PASS=$((PASS+1))
else
  red   "FAIL  rollback did not clear proof body"
  FAIL=$((FAIL+1))
fi

echo
bold "── SUMMARY ───────────────────────────────────────────────"
echo "PASS: $PASS   FAIL: $FAIL"
if [[ "$FAIL" == 0 ]]; then
  green "ALL GATES HELD"
  exit 0
else
  red "AT LEAST ONE GATE FAILED — see above"
  exit 1
fi

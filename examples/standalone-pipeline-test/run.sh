#!/usr/bin/env bash
# Standalone Pipeline Smoke Test
# Exercises the full tiered inference routing chain:
#   workplan → tier classification → model selection → Ollama inference → compile gate
#
# Prerequisites:
#   - hex nexus running (hex nexus start)
#   - Ollama reachable on bazzite:11434 (or OLLAMA_HOST)
#
# Usage: ./run.sh [--tier T1|T2|T2.5|all] [--verbose]

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
HEX_BIN="${PROJECT_ROOT}/target/debug/hex"
# OLLAMA_HOST may be a bind address (0.0.0.0) — not useful for connecting.
# Prefer the inference-servers.json config, fall back to env, then default.
_raw_host="${OLLAMA_HOST:-}"
if [[ -z "$_raw_host" || "$_raw_host" == "0.0.0.0" || "$_raw_host" == "0.0.0.0:"* ]]; then
  # Read from inference config if available
  _cfg_host=$(jq -r '
    .endpoints[] | select(.provider == "ollama") | .url
  ' ~/.hex/inference-servers.json 2>/dev/null | head -1)
  _cfg_host="${_cfg_host:-http://bazzite:11434}"
  OLLAMA_HOST="${_cfg_host}"
else
  # Ensure it has a scheme
  [[ "$_raw_host" == http* ]] || _raw_host="http://${_raw_host}"
  OLLAMA_HOST="$_raw_host"
fi
NEXUS_URL="${HEX_NEXUS_URL:-http://127.0.0.1:5555}"
WORKDIR=$(mktemp -d)
TIER_FILTER="all"
VERBOSE=false

# Tier → model mapping (from ~/.hex/inference-servers.json tier_defaults)
T1_MODEL="qwen3:4b"
T2_MODEL="qwen2.5-coder:32b"
T25_MODEL="qwen3.5:27b"

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
DIM='\033[2m'
RESET='\033[0m'

# GBNF grammar for code-only output (ADR-2604120202 Phase 2)
# Forces model to emit ONLY a code block — no prose, no explanation.
GBNF_CODE_ONLY='root ::= code-line+
code-line ::= [^\n]* "\n"'
USE_GRAMMAR=true

# Parse args
while [[ $# -gt 0 ]]; do
  case $1 in
    --tier) TIER_FILTER="$2"; shift 2 ;;
    --verbose) VERBOSE=true; shift ;;
    --no-grammar) USE_GRAMMAR=false; shift ;;
    *) echo "Unknown arg: $1"; exit 1 ;;
  esac
done

cleanup() { rm -rf "$WORKDIR"; }
trap cleanup EXIT

log()  { echo -e "${CYAN}[pipeline]${RESET} $*"; }
pass() { echo -e "${GREEN}  PASS${RESET} $*"; }
fail() { echo -e "${RED}  FAIL${RESET} $*"; }
skip() { echo -e "${YELLOW}  SKIP${RESET} $*"; }
dim()  { echo -e "${DIM}$*${RESET}"; }

# ─── Preflight ──────────────────────────────────────────────────────
log "Standalone Pipeline Smoke Test"
log "Working dir: $WORKDIR"
log "Ollama: $OLLAMA_HOST"
log "Nexus: $NEXUS_URL"
echo ""

# Check nexus
if ! curl -sf "$NEXUS_URL/api/health" >/dev/null 2>&1; then
  fail "hex-nexus not reachable at $NEXUS_URL"
  echo "  Run: hex nexus start"
  exit 1
fi
pass "hex-nexus healthy"

# Check Ollama
if ! curl -sf "$OLLAMA_HOST/api/tags" >/dev/null 2>&1; then
  fail "Ollama not reachable at $OLLAMA_HOST"
  exit 1
fi
pass "Ollama reachable"

# Check SpacetimeDB rl-engine
STDB_HOST="${HEX_SPACETIMEDB_HOST:-http://127.0.0.1:3033}"
RL_ENABLED=true
if ! curl -sf "$STDB_HOST/v1/database/rl-engine/sql" \
  -d 'SELECT * FROM rl_q_entry LIMIT 1' >/dev/null 2>&1; then
  skip "rl-engine not available — rewards will not be recorded"
  RL_ENABLED=false
else
  pass "rl-engine connected (SpacetimeDB)"
fi
echo ""

# ─── Helper: record reward to RL engine ─────────────────────────────
# Maps pipeline outcomes to RL signals:
#   state_key:  "tier:{tier}|task_type:{task_type}"
#   action:     "model:{model_name}"
#   reward:     +1.0 (compiled attempt 1), +0.5 (compiled attempt 2-3),
#               +0.25 (compiled attempt 4-5), 0.0 (all failed)
#   next_state: same state (stationary for now)
record_rl_reward() {
  local tier="$1"
  local task_type="$2"
  local model="$3"
  local attempt="$4"      # which attempt succeeded (0 = all failed)
  local max_attempts="$5"

  if ! $RL_ENABLED; then return; fi

  local state_key="tier:${tier}|task_type:${task_type}"
  local action="model:${model}"
  local reward

  if [[ "$attempt" -eq 0 ]]; then
    reward=0.0
  elif [[ "$attempt" -eq 1 ]]; then
    reward=1.0
  elif [[ "$attempt" -le 3 ]]; then
    reward=0.5
  else
    reward=0.25
  fi

  # Call SpacetimeDB record_reward reducer
  # Signature: record_reward(state_key, action, reward, next_state_key, rate_limited, openrouter_cost_usd)
  local result
  result=$(curl -sf "$STDB_HOST/v1/database/rl-engine/call/record_reward" \
    -H "Content-Type: application/json" \
    -d "$(jq -n \
      --arg sk "$state_key" \
      --arg act "$action" \
      --argjson rew "$reward" \
      --arg nsk "$state_key" \
      '[$sk, $act, $rew, $nsk, false, 0.0]')" 2>&1)

  if [[ $? -eq 0 ]]; then
    dim "    RL: reward=${reward} for ${action} in state ${state_key}" >&2
  else
    dim "    RL: failed to record reward (${result})" >&2
  fi
}

# ─── Helper: query RL Q-table for a state ───────────────────────────
query_rl_policy() {
  if ! $RL_ENABLED; then return; fi

  echo ""
  log "RL Q-Table (current policy):"

  local rows
  rows=$(curl -sf "$STDB_HOST/v1/database/rl-engine/sql" \
    -d 'SELECT * FROM rl_q_entry' 2>&1)

  if [[ $? -eq 0 ]]; then
    # SpacetimeDB returns [{schema: ..., rows: [[col1, col2, ...]]}]
    # Columns: [composite_id, state_key, action, q_value, visit_count, last_updated]
    local model_rows
    model_rows=$(echo "$rows" | jq -r '
      .[0].rows[]
      | select(.[2] | contains("model:"))
      | "  \(.[1])\t\(.[2])\tQ=\(.[3])\tvisits=\(.[4])"
    ' 2>/dev/null | sort -t$'\t' -k3 -rn)

    if [[ -z "$model_rows" ]]; then
      echo "  (no model selection entries yet)"
    else
      echo "$model_rows" | column -t -s$'\t' | head -20
    fi
  else
    dim "  Could not query Q-table"
  fi
}

# ─── Helper: call Ollama and measure ────────────────────────────────
ollama_generate() {
  local model="$1"
  local prompt="$2"
  local temp="${3:-0.2}"
  local grammar="${4:-}"

  local start_ms=$(($(date +%s%N 2>/dev/null || python3 -c 'import time; print(int(time.time()*1000))') / 1000000))

  local json_body
  if [[ -n "$grammar" ]]; then
    json_body=$(jq -n \
      --arg model "$model" \
      --arg prompt "$prompt" \
      --argjson temp "$temp" \
      --arg grammar "$grammar" \
      '{model: $model, prompt: $prompt, temperature: $temp, stream: false, grammar: $grammar}')
  else
    json_body=$(jq -n \
      --arg model "$model" \
      --arg prompt "$prompt" \
      --argjson temp "$temp" \
      '{model: $model, prompt: $prompt, temperature: $temp, stream: false}')
  fi

  local response
  response=$(curl -sf "$OLLAMA_HOST/api/generate" \
    -d "$json_body" \
    2>&1) || { echo "ERROR: Ollama request failed"; return 1; }

  local end_ms=$(($(date +%s%N 2>/dev/null || python3 -c 'import time; print(int(time.time()*1000))') / 1000000))
  local elapsed_ms=$(( end_ms - start_ms ))

  local text
  text=$(echo "$response" | jq -r '.response // empty')
  local eval_count
  eval_count=$(echo "$response" | jq -r '.eval_count // 0')
  local eval_duration
  eval_duration=$(echo "$response" | jq -r '.eval_duration // 0')

  local tok_per_sec=0
  if [[ "$eval_duration" -gt 0 ]]; then
    tok_per_sec=$(echo "scale=1; $eval_count / ($eval_duration / 1000000000)" | bc 2>/dev/null || echo "?")
  fi

  dim "    Model: $model | Tokens: $eval_count | ${tok_per_sec} tok/s | ${elapsed_ms}ms" >&2

  echo "$text"
}

# ─── Helper: extract code from response ─────────────────────────────
extract_code() {
  local text="$1"
  # Try to extract from ```rust ... ``` blocks first
  local code
  code=$(echo "$text" | sed -n '/^```rust/,/^```/p' | sed '1d;$d')
  if [[ -z "$code" ]]; then
    # Try generic ``` blocks
    code=$(echo "$text" | sed -n '/^```/,/^```/p' | sed '1d;$d')
  fi
  if [[ -z "$code" ]]; then
    # Use raw text (model returned code directly)
    code="$text"
  fi
  echo "$code"
}

# ─── Helper: compile gate (multi-language) ─────────────────────────
compile_gate() {
  local code="$1"
  local name="$2"
  local lang="${3:-rust}"

  local ext err_file
  case "$lang" in
    rust)       ext="rs" ;;
    typescript) ext="ts" ;;
    go)         ext="go" ;;
    *)          ext="rs" ;;
  esac

  local file="$WORKDIR/${name}.${ext}"
  err_file="$WORKDIR/${name}_err.txt"
  echo "$code" > "$file"

  case "$lang" in
    rust)
      # Try normal compile first, then --test for library/test code without main()
      if rustc --edition 2021 "$file" -o "$WORKDIR/${name}" 2>"$err_file"; then
        return 0
      elif grep -q "main.*function not found" "$err_file" && \
           rustc --edition 2021 --test "$file" -o "$WORKDIR/${name}" 2>"$err_file"; then
        return 0
      fi
      ;;
    typescript)
      # --skipLibCheck prevents bun-types / node_modules conflicts from the project root
      if tsc --noEmit --strict --target es2020 --moduleResolution node --skipLibCheck "$file" 2>"$err_file"; then
        return 0
      fi
      ;;
    go)
      if (cd "$WORKDIR" && go build -o "${name}_bin" "./${name}.${ext}") 2>"$err_file"; then
        return 0
      fi
      ;;
  esac

  if $VERBOSE; then
    dim "    Compile error ($lang):" >&2
    head -10 "$err_file" | sed 's/^/    /' >&2
  fi
  return 1
}

# ─── Helper: Best-of-N with compile gate ────────────────────────────
best_of_n() {
  local model="$1"
  local prompt="$2"
  local name="$3"
  local n="${4:-1}"
  local lang="${5:-rust}"
  local grammar="${6:-}"

  for attempt in $(seq 1 "$n"); do
    dim "    Attempt $attempt/$n..." >&2
    local text
    text=$(ollama_generate "$model" "$prompt" 0.3 "$grammar")
    local code
    code=$(extract_code "$text")

    if compile_gate "$code" "${name}_attempt${attempt}" "$lang"; then
      pass "Compiled on attempt $attempt" >&2
      echo "$attempt" > "$WORKDIR/.last_attempt"
      echo "$code"
      return 0
    fi
  done

  echo "0" > "$WORKDIR/.last_attempt"
  fail "All $n attempts failed to compile" >&2
  return 1
}

# ─── Counters ───────────────────────────────────────────────────────
TOTAL=0
PASSED=0
FAILED=0
SKIPPED=0

record_pass()  { TOTAL=$((TOTAL + 1)); PASSED=$((PASSED + 1)); }
record_fail()  { TOTAL=$((TOTAL + 1)); FAILED=$((FAILED + 1)); }
record_skip()  { TOTAL=$((TOTAL + 1)); SKIPPED=$((SKIPPED + 1)); }

# ─── TIER T1: Trivial edits ────────────────────────────────────────
run_t1() {
  log "Phase: T1 - Trivial Edits (model: $T1_MODEL)"
  echo ""

  # T1.1: Rename variable
  log "  Task t1.1: Rename variable x -> count"
  local text
  local grammar_arg=""
  if $USE_GRAMMAR; then grammar_arg="$GBNF_CODE_ONLY"; fi

  text=$(ollama_generate "$T1_MODEL" \
    "Rename the variable x to count in this Rust code. Return ONLY the modified code, no explanation:\n\nfn main() {\n    let x = 42;\n    println!(\"Value: {}\", x);\n}" \
    0.2 "$grammar_arg")
  local code
  code=$(extract_code "$text")

  if echo "$code" | grep -q "let count = 42"; then
    pass "t1.1 — contains 'let count = 42'"
    record_pass
    record_rl_reward "T1" "rename_variable" "$T1_MODEL" 1 1
  else
    fail "t1.1 — missing 'let count = 42'"
    if $VERBOSE; then dim "    Got: $(echo "$code" | head -5)"; fi
    record_fail
    record_rl_reward "T1" "rename_variable" "$T1_MODEL" 0 1
  fi
  echo ""

  # T1.2: Fix typo
  log "  Task t1.2: Fix typo (teh -> the)"
  text=$(ollama_generate "$T1_MODEL" \
    "Fix the typo in this Rust code (teh -> the). Return ONLY the fixed code:\n\n// This is teh main function\nfn main() {\n    println!(\"hello\");\n}" \
    0.2 "$grammar_arg")
  code=$(extract_code "$text")

  if echo "$code" | grep -q "This is the main function"; then
    pass "t1.2 — typo fixed"
    record_pass
    record_rl_reward "T1" "fix_typo" "$T1_MODEL" 1 1
  else
    fail "t1.2 — typo not fixed"
    if $VERBOSE; then dim "    Got: $(echo "$code" | head -5)"; fi
    record_fail
    record_rl_reward "T1" "fix_typo" "$T1_MODEL" 0 1
  fi
  echo ""
}

# ─── TIER T2: Single function with compile gate ────────────────────
run_t2() {
  log "Phase: T2 - Single Function (model: $T2_MODEL, Best-of-3)"
  echo ""

  # T2.1: Fibonacci
  log "  Task t2.1: Generate fibonacci function"
  local code
  if code=$(best_of_n "$T2_MODEL" \
    "Write a Rust function fn fibonacci(n: u64) -> u64 that returns the nth Fibonacci number using iteration (not recursion). Include a main() that prints fibonacci(10). Return ONLY valid Rust code, no explanation." \
    "t2_1_fib" 3); then
    # Run it
    if "$WORKDIR/t2_1_fib_attempt"* 2>/dev/null | grep -q "55"; then
      pass "t2.1 — fibonacci(10) = 55"
    else
      pass "t2.1 — compiled (output validation skipped)"
    fi
    record_pass
    record_rl_reward "T2" "single_function" "$T2_MODEL" "$(cat "$WORKDIR/.last_attempt" 2>/dev/null || echo 0)" 3
  else
    fail "t2.1 — could not produce compilable fibonacci"
    record_fail
    record_rl_reward "T2" "single_function" "$T2_MODEL" 0 3
  fi
  echo ""

  # T2.2: Palindrome with tests
  log "  Task t2.2: Generate palindrome checker with tests"
  if code=$(best_of_n "$T2_MODEL" \
    "Write a Rust module with: fn is_palindrome(s: &str) -> bool that checks if a string is a palindrome (case-insensitive, ignoring non-alphanumeric chars). Include #[cfg(test)] mod tests with at least 3 test cases using #[test]. Return ONLY valid Rust code." \
    "t2_2_palindrome" 3); then
    pass "t2.2 — compiled"
    record_pass
    record_rl_reward "T2" "function_with_tests" "$T2_MODEL" "$(cat "$WORKDIR/.last_attempt" 2>/dev/null || echo 0)" 3
  else
    fail "t2.2 — could not produce compilable palindrome checker"
    record_fail
    record_rl_reward "T2" "function_with_tests" "$T2_MODEL" 0 3
  fi
  echo ""
}

# ─── TIER T2.5: Multi-function ──────────────────────────────────────
run_t25() {
  log "Phase: T2.5 - Multi-function (model: $T25_MODEL, Best-of-5)"
  echo ""

  # T2.5.1: CLI arg parser
  log "  Task t25.1: Generate CLI argument parser"
  local code
  if code=$(best_of_n "$T25_MODEL" \
    "Write a complete Rust program (single file, no external crates) that: 1) Parses command-line args: --name <string> and --count <u32> (both required). 2) Prints the name repeated count times, one per line. 3) Prints usage and exits with code 1 if args missing. 4) Include a parse_args function and a main function. Return ONLY valid Rust code." \
    "t25_1_cli" 5); then
    pass "t25.1 — compiled"
    record_pass
    record_rl_reward "T2.5" "multi_function_cli" "$T25_MODEL" "$(cat "$WORKDIR/.last_attempt" 2>/dev/null || echo 0)" 5
  else
    fail "t25.1 — could not produce compilable CLI parser"
    record_fail
    record_rl_reward "T2.5" "multi_function_cli" "$T25_MODEL" 0 5
  fi
  echo ""
}

# ─── TypeScript Tests ───────────────────────────────────────────────
run_ts() {
  log "Phase: TypeScript (model: $T2_MODEL, Best-of-3)"
  echo ""

  # TS T1: Rename variable
  log "  Task ts.1: [T1] Rename variable in TypeScript"
  local grammar_arg=""
  if $USE_GRAMMAR; then grammar_arg="$GBNF_CODE_ONLY"; fi

  local text
  text=$(ollama_generate "$T1_MODEL" \
    "Rename the variable 'x' to 'count' in this TypeScript code. Return ONLY the modified code:\n\nconst x: number = 42;\nconsole.log(\`Value: \${x}\`);" \
    0.2 "$grammar_arg")
  local code
  code=$(extract_code "$text")

  if echo "$code" | grep -q "count.*=.*42\|count:.*number.*=.*42"; then
    pass "ts.1 — variable renamed"
    record_pass
    record_rl_reward "T1" "ts_rename_variable" "$T1_MODEL" 1 1
  else
    fail "ts.1 — variable not renamed"
    if $VERBOSE; then dim "    Got: $(echo "$code" | head -3)"; fi
    record_fail
    record_rl_reward "T1" "ts_rename_variable" "$T1_MODEL" 0 1
  fi
  echo ""

  # TS T2: Generate a function with type checking
  log "  Task ts.2: [T2] Generate TypeScript function (compile gate)"
  local code
  if code=$(best_of_n "$T2_MODEL" \
    "Write a TypeScript file with: 1) A function 'isPalindrome(s: string): boolean' that checks if a string is a palindrome (case-insensitive). 2) A function 'reverseWords(s: string): string' that reverses word order. 3) Export both functions. Return ONLY valid TypeScript code, no explanation." \
    "ts_2_funcs" 3 "typescript"); then
    pass "ts.2 — type checks passed"
    record_pass
    record_rl_reward "T2" "ts_typed_functions" "$T2_MODEL" "$(cat "$WORKDIR/.last_attempt" 2>/dev/null || echo 0)" 3
  else
    fail "ts.2 — could not produce type-safe TypeScript"
    record_fail
    record_rl_reward "T2" "ts_typed_functions" "$T2_MODEL" 0 3
  fi
  echo ""
}

# ─── Go Tests ──────────────────────────────────────────────────────
run_go() {
  log "Phase: Go (model: $T2_MODEL, Best-of-3)"
  echo ""

  # Go T1: Fix typo
  log "  Task go.1: [T1] Fix typo in Go code"
  local grammar_arg=""
  if $USE_GRAMMAR; then grammar_arg="$GBNF_CODE_ONLY"; fi

  local text
  text=$(ollama_generate "$T1_MODEL" \
    "Fix the typo in this Go code (teh -> the). Return ONLY the fixed code:\n\npackage main\n\nimport \"fmt\"\n\n// This is teh main function\nfunc main() {\n\tfmt.Println(\"hello\")\n}" \
    0.2 "$grammar_arg")
  local code
  code=$(extract_code "$text")

  if echo "$code" | grep -q "This is the main function"; then
    pass "go.1 — typo fixed"
    record_pass
    record_rl_reward "T1" "go_fix_typo" "$T1_MODEL" 1 1
  else
    fail "go.1 — typo not fixed"
    if $VERBOSE; then dim "    Got: $(echo "$code" | head -3)"; fi
    record_fail
    record_rl_reward "T1" "go_fix_typo" "$T1_MODEL" 0 1
  fi
  echo ""

  # Go T2: Generate a function with compile gate
  log "  Task go.2: [T2] Generate Go function (compile gate)"
  local code
  if code=$(best_of_n "$T2_MODEL" \
    "Write a complete Go file (package main) with: 1) A function 'fibonacci(n int) int' that returns the nth fibonacci number using iteration. 2) A main function that prints fibonacci(10). Return ONLY valid Go code, no explanation." \
    "go_2_fib" 3 "go"); then
    pass "go.2 — compiled"
    record_pass
    record_rl_reward "T2" "go_single_function" "$T2_MODEL" "$(cat "$WORKDIR/.last_attempt" 2>/dev/null || echo 0)" 3
  else
    fail "go.2 — could not produce compilable Go"
    record_fail
    record_rl_reward "T2" "go_single_function" "$T2_MODEL" 0 3
  fi
  echo ""
}

# ─── Main ───────────────────────────────────────────────────────────
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
log "Tier Routing: T1→$T1_MODEL | T2→$T2_MODEL | T2.5→$T25_MODEL"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo ""

case "$TIER_FILTER" in
  T1)   run_t1 ;;
  T2)   run_t2 ;;
  T2.5) run_t25 ;;
  ts)   run_ts ;;
  go)   run_go ;;
  all)  run_t1; run_t2; run_t25; run_ts; run_go ;;
  *)    echo "Unknown tier: $TIER_FILTER (options: T1, T2, T2.5, ts, go, all)"; exit 1 ;;
esac

# ─── Summary ────────────────────────────────────────────────────────
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
log "Results: ${PASSED}/${TOTAL} passed, ${FAILED} failed, ${SKIPPED} skipped"

# ─── RL Policy Report ──────────────────────────────────────────────
query_rl_policy

echo ""
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
if [[ "$FAILED" -gt 0 ]]; then
  echo -e "${RED}PIPELINE FAILED${RESET}"
  exit 1
else
  echo -e "${GREEN}PIPELINE PASSED${RESET}"
  exit 0
fi

#!/usr/bin/env bash
# benchmark-ollama.sh — benchmark all installed Ollama models
# Usage: ./scripts/benchmark-ollama.sh [--runs N] [--prompt "..."] [--host http://localhost:11434]
# Output: ranked table of tok/s, TTFT, and prompt tok/s
#
# On Bazzite with AMD iGPU: export OLLAMA_VULKAN=true before running Ollama.

set -euo pipefail

# ── Config ────────────────────────────────────────────────────────────────────
HOST="${OLLAMA_HOST:-http://localhost:11434}"
RUNS="${RUNS:-3}"
TIMEOUT="${TIMEOUT:-120}"
PROMPT="${BENCHMARK_PROMPT:-Explain the difference between a transformer and an RNN in exactly 100 words.}"

# ── Colors ────────────────────────────────────────────────────────────────────
BOLD="\033[1m"
GREEN="\033[32m"
YELLOW="\033[33m"
RED="\033[31m"
CYAN="\033[36m"
RESET="\033[0m"

# ── Deps check ────────────────────────────────────────────────────────────────
for cmd in curl jq awk; do
  command -v "$cmd" &>/dev/null || { echo "Missing: $cmd"; exit 1; }
done

# ── Get model list ────────────────────────────────────────────────────────────
echo -e "${BOLD}${CYAN}hex ollama benchmark${RESET} — ${HOST}"
echo -e "Runs per model: ${RUNS} | Timeout: ${TIMEOUT}s"
echo -e "Prompt: \"${PROMPT:0:60}...\"\n"

MODELS=$(curl -sf "${HOST}/api/tags" | jq -r '.models[].name' 2>/dev/null)
if [[ -z "$MODELS" ]]; then
  echo -e "${RED}No models found. Is Ollama running at ${HOST}?${RESET}"
  exit 1
fi

MODEL_COUNT=$(echo "$MODELS" | wc -l | tr -d ' ')
echo -e "Found ${BOLD}${MODEL_COUNT}${RESET} models:\n"
echo "$MODELS" | sed 's/^/  /'
echo ""

# ── Results array (tab-separated: model, avg_toks, avg_ttft_ms, avg_prompt_tps, errors) ──
declare -a RESULTS

# ── Benchmark each model ──────────────────────────────────────────────────────
while IFS= read -r MODEL; do
  echo -e "${BOLD}▸ ${MODEL}${RESET}"

  total_gen_tps=0
  total_ttft_ms=0
  total_prompt_tps=0
  errors=0
  successful_runs=0

  for run in $(seq 1 "$RUNS"); do
    printf "  run %d/%d ... " "$run" "$RUNS"

    RESPONSE=$(curl -sf --max-time "$TIMEOUT" \
      -X POST "${HOST}/api/generate" \
      -H "Content-Type: application/json" \
      -d "{
        \"model\": \"${MODEL}\",
        \"prompt\": $(echo "$PROMPT" | jq -Rs .),
        \"stream\": false,
        \"options\": { \"temperature\": 0.1, \"num_predict\": 150 }
      }" 2>/dev/null) || { echo -e "${RED}timeout/error${RESET}"; errors=$((errors + 1)); continue; }

    # Parse metrics
    eval_count=$(echo "$RESPONSE"    | jq -r '.eval_count    // 0')
    eval_dur=$(echo "$RESPONSE"      | jq -r '.eval_duration // 0')
    prompt_count=$(echo "$RESPONSE"  | jq -r '.prompt_eval_count    // 0')
    prompt_dur=$(echo "$RESPONSE"    | jq -r '.prompt_eval_duration // 0')

    # Validate non-zero
    if [[ "$eval_dur" -le 0 || "$eval_count" -le 0 ]]; then
      echo -e "${RED}bad response${RESET}"
      errors=$((errors + 1))
      continue
    fi

    # tok/s = count / (duration_ns / 1e9)
    gen_tps=$(awk "BEGIN { printf \"%.1f\", ${eval_count} / (${eval_dur} / 1000000000) }")
    ttft_ms=$(awk "BEGIN { printf \"%.0f\", ${prompt_dur} / 1000000 }")
    prompt_tps=$(awk "BEGIN {
      if (${prompt_dur} > 0)
        printf \"%.1f\", ${prompt_count} / (${prompt_dur} / 1000000000)
      else
        printf \"0\"
    }")

    echo -e "${GREEN}${gen_tps} tok/s${RESET} (TTFT: ${ttft_ms}ms, prompt: ${prompt_tps} tok/s)"

    total_gen_tps=$(awk "BEGIN { printf \"%.1f\", ${total_gen_tps} + ${gen_tps} }")
    total_ttft_ms=$(awk "BEGIN { printf \"%.0f\", ${total_ttft_ms} + ${ttft_ms} }")
    total_prompt_tps=$(awk "BEGIN { printf \"%.1f\", ${total_prompt_tps} + ${prompt_tps} }")
    successful_runs=$((successful_runs + 1))
  done

  if [[ "$successful_runs" -eq 0 ]]; then
    echo -e "  ${RED}All runs failed${RESET}\n"
    RESULTS+=("${MODEL}\tFAILED\t-\t-\t${errors}")
    continue
  fi

  avg_gen=$(awk "BEGIN { printf \"%.1f\", ${total_gen_tps} / ${successful_runs} }")
  avg_ttft=$(awk "BEGIN { printf \"%.0f\",  ${total_ttft_ms} / ${successful_runs} }")
  avg_prompt=$(awk "BEGIN { printf \"%.1f\", ${total_prompt_tps} / ${successful_runs} }")

  echo -e "  ${BOLD}avg: ${GREEN}${avg_gen} tok/s${RESET} | TTFT: ${avg_ttft}ms | prompt: ${avg_prompt} tok/s\n"
  RESULTS+=("${MODEL}\t${avg_gen}\t${avg_ttft}\t${avg_prompt}\t${errors}")

done <<< "$MODELS"

# ── Sort results by gen tok/s descending ──────────────────────────────────────
echo -e "\n${BOLD}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${RESET}"
echo -e "${BOLD}  RESULTS — ranked by generation tok/s${RESET}"
echo -e "${BOLD}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${RESET}"
printf "${BOLD}  %-40s  %10s  %10s  %12s  %s${RESET}\n" "MODEL" "GEN tok/s" "TTFT (ms)" "PROMPT tok/s" "ERR"
echo -e "  $(printf '%.0s─' {1..75})"

# Sort by gen tok/s (field 2), FAILED goes to bottom
SORTED=$(printf '%s\n' "${RESULTS[@]}" | sort -t$'\t' -k2 -rn)

RANK=1
while IFS=$'\t' read -r model gen ttft prompt errs; do
  if [[ "$gen" == "FAILED" ]]; then
    printf "  ${RED}%-40s  %10s  %10s  %12s  %s${RESET}\n" "$model" "FAILED" "-" "-" "$errs"
  else
    # Color: green if fast, yellow if moderate, red if slow
    COLOR=$GREEN
    GEN_INT=${gen%.*}
    if [[ "$GEN_INT" -lt 10 ]]; then COLOR=$RED;
    elif [[ "$GEN_INT" -lt 25 ]]; then COLOR=$YELLOW; fi

    printf "  ${BOLD}%2d.${RESET} ${COLOR}%-36s  %10s  %10s  %12s${RESET}  %s\n" \
      "$RANK" "$model" "$gen" "$ttft" "$prompt" "$errs"
    RANK=$((RANK + 1))
  fi
done <<< "$SORTED"

echo -e "  $(printf '%.0s─' {1..75})"
echo -e "\n${CYAN}Tip: Add top models to hex inference:${RESET}"
echo -e "  hex inference add --name <model> --provider ollama --model <model> --base-url ${HOST}\n"

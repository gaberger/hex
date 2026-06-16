#!/usr/bin/env bash
#
# train-lora.sh — OFFLINE dev tooling for hex LoRA idiom experts (ADR-2606161300).
#
# This is the ONLY non-hex surface in the LoRA pipeline, and it is deliberately fenced
# as build/dev tooling (CLAUDE.md "No Runtime Scripts" exception for scripts/). It does
# NOT run at runtime, the daemon never invokes it, and it touches NO correctness gate.
# It consumes a corpus the daemon already produced and emits a GGUF adapter the daemon
# can later SERVE — training is an offline step precisely because gradient training of a
# 24-32B base is neither in-process Rust nor a daemon responsibility, and would OOM a
# 16GB-GPU/30GB-RAM box under load (project_qwen_next_hardware_ceiling).
#
# Pipeline (Phase 0 -> Phase 1, all gated by the bench harness, never by training loss):
#
#   1. hex inference corpus build <expert>      # daemon writes .hex/corpus/<expert>/corpus.jsonl
#   2. scripts/train-lora.sh <expert>           # THIS script -> GGUF adapter (offline)
#   3. hex inference adapter register ...        # daemon records (base, tier, expert, corpus-version)
#   4. (serving) base+adapter rides the tier     # automatic; bare base if absent/disabled
#   5. hex inference adapter evaluate <expert>   # bench gate: promote ONLY on acceptance lift
#
# The adapter is an IDIOM PRIOR, never an enforcement mechanism (ADR-2606161300 §1):
# removing it must not weaken any gate. `hex analyze`, specs, and the best-of-N compile
# gate remain the sole arbiters of correctness.
#
# 16GB-GPU caveat: a rank-4 LoRA on a 24-32B base is feasible but tight. Prefer a
# quantized base (q4) or off-box training; respect the resource governor (ADR-2606080915).
# This script only prints the recommended training invocation by default — pass --run to
# actually launch a configured trainer.

set -euo pipefail

readonly LORA_RANK=4          # DMoE rank (arXiv:2606.14243)
readonly LORA_ALPHA=16        # DMoE alpha
readonly DEFAULT_BASE="qwen2.5-coder:32b"
readonly DEFAULT_TIER=2

usage() {
  cat <<'EOF'
Usage: scripts/train-lora.sh <expert> [options]

Consumes .hex/corpus/<expert>/corpus.jsonl and produces a GGUF LoRA adapter, then
prints the `hex inference adapter register ...` command to wire it in.

Arguments:
  <expert>                Expert name (e.g. hex-boundaries). Must have a built corpus.

Options:
  --base <model>          Base model to fine-tune       (default: qwen2.5-coder:32b)
  --tier <n>              Tier the adapter serves        (default: 2)
  --out <path>            Output GGUF adapter path       (default: .hex/adapters/<expert>.gguf)
  --rank <n>              LoRA rank                      (default: 4, per DMoE)
  --alpha <n>             LoRA alpha                     (default: 16, per DMoE)
  --run                   Actually launch the trainer (default: print the invocation only)
  -h, --help              Show this help and the full Phase 0->1 loop

Pipeline:
  1. hex inference corpus build <expert>
  2. scripts/train-lora.sh <expert>
  3. hex inference adapter register --expert <expert> --base <base> --tier <n> \
       --artifact <out.gguf> --corpus-version <v>
  4. (serving) base+adapter rides the tier automatically
  5. hex inference adapter evaluate <expert>   # promote ONLY on bench-measured lift

LoRA attaches to the final-layer FFN (DMoE) to preserve KV-cache reuse across the
ReAct loop. The adapter is an idiom prior, never enforcement (ADR-2606161300 §1).
EOF
}

main() {
  if [[ $# -lt 1 ]]; then
    usage
    exit 1
  fi
  case "$1" in
    -h | --help)
      usage
      exit 0
      ;;
  esac

  local expert="$1"
  shift

  local base="$DEFAULT_BASE"
  local tier="$DEFAULT_TIER"
  local out=""
  local rank="$LORA_RANK"
  local alpha="$LORA_ALPHA"
  local do_run=0

  while [[ $# -gt 0 ]]; do
    case "$1" in
      --base)  base="$2";  shift 2 ;;
      --tier)  tier="$2";  shift 2 ;;
      --out)   out="$2";   shift 2 ;;
      --rank)  rank="$2";  shift 2 ;;
      --alpha) alpha="$2"; shift 2 ;;
      --run)   do_run=1;   shift ;;
      *)
        echo "error: unknown option '$1'" >&2
        usage
        exit 1
        ;;
    esac
  done

  local repo_root corpus manifest out_path
  repo_root="${HEX_REPO_ROOT:-$(pwd)}"
  corpus="${repo_root}/.hex/corpus/${expert}/corpus.jsonl"
  manifest="${repo_root}/.hex/corpus/${expert}/manifest.json"
  out_path="${out:-${repo_root}/.hex/adapters/${expert}.gguf}"

  if [[ ! -f "$corpus" ]]; then
    echo "error: corpus not found: ${corpus}" >&2
    echo "  run first:  hex inference corpus build ${expert}" >&2
    exit 1
  fi

  # The corpus_version stamp ties the adapter to the exact corpus it trained on, so the
  # daemon can later flag the adapter stale when the source ADRs/specs change.
  local corpus_version="unknown"
  if [[ -f "$manifest" ]]; then
    corpus_version="$(grep -o '"content_hash"[[:space:]]*:[[:space:]]*"[^"]*"' "$manifest" \
      | head -n1 | sed 's/.*"\([^"]*\)"$/\1/')"
    corpus_version="${corpus_version:-unknown}"
  fi

  local records
  records="$(wc -l <"$corpus" | tr -d ' ')"

  echo "hex LoRA training (offline dev tooling) — ADR-2606161300"
  echo "  expert:         ${expert}"
  echo "  base:           ${base}"
  echo "  tier:           ${tier}"
  echo "  corpus:         ${corpus} (${records} records)"
  echo "  corpus_version: ${corpus_version}"
  echo "  rank/alpha:     ${rank}/${alpha} (final-layer FFN attachment)"
  echo "  output:         ${out_path}"
  echo

  mkdir -p "$(dirname "$out_path")"

  # The concrete trainer is environment-specific (peft/unsloth/llama.cpp finetune).
  # We pin the DMoE hyperparameters and target modules. The default trainer is the
  # bundled QLoRA toolchain (scripts/lora/run.sh, set up by scripts/lora/setup.sh);
  # override HEX_LORA_TRAINER to point at a different one. We never silently fabricate
  # a trainer — if the bundled one isn't set up, --run fails loudly.
  local default_trainer="${repo_root}/scripts/lora/run.sh"
  local trainer="${HEX_LORA_TRAINER:-${default_trainer}}"
  local register_cmd="hex inference adapter register --expert ${expert} --base ${base} --tier ${tier} --artifact ${out_path} --corpus-version ${corpus_version}"

  if [[ "$do_run" -eq 1 ]]; then
    if [[ -z "$trainer" ]]; then
      echo "error: --run requires HEX_LORA_TRAINER to point at a LoRA trainer binary" >&2
      echo "  (e.g. a peft/unsloth/llama.cpp finetune wrapper that reads --data/--out)" >&2
      exit 1
    fi
    echo "Launching trainer: ${trainer}"
    "$trainer" \
      --base "$base" \
      --data "$corpus" \
      --out "$out_path" \
      --rank "$rank" \
      --alpha "$alpha" \
      --target-modules "final_ffn"
    echo
    echo "Adapter written: ${out_path}"
    echo "Now register it with hex-nexus:"
    echo "  ${register_cmd}"
  else
    echo "Dry print (no --run). To train, set HEX_LORA_TRAINER and re-run with --run."
    echo "After training, register the adapter with:"
    echo "  ${register_cmd}"
  fi
}

main "$@"

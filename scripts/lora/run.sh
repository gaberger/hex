#!/usr/bin/env bash
#
# run.sh — the HEX_LORA_TRAINER entrypoint (ADR-2606161300, offline dev tooling).
#
# scripts/train-lora.sh invokes "$HEX_LORA_TRAINER --base .. --data .. --out .. --rank ..
# --alpha .. --target-modules .." when launched with --run. This wrapper activates the
# venv created by setup.sh and forwards to train_lora.py. It also exposes a `smoke`
# shortcut that validates the whole corpus->train->GGUF path on CPU with a tiny model.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
readonly SCRIPT_DIR
readonly VENV_DIR="${SCRIPT_DIR}/.venv"
readonly PY="${SCRIPT_DIR}/train_lora.py"

if [[ ! -d "${VENV_DIR}" ]]; then
  echo "error: venv missing — run scripts/lora/setup.sh first" >&2
  exit 1
fi
# shellcheck disable=SC1091
source "${VENV_DIR}/bin/activate"

# `smoke`: build a tiny throwaway corpus and run the pipeline end-to-end on CPU.
if [[ "${1:-}" == "smoke" ]]; then
  tmp="$(mktemp -d)"
  trap 'rm -rf "${tmp}"' EXIT
  cat >"${tmp}/corpus.jsonl" <<'EOF'
{"instruction":"State the hex rule about adapters importing adapters.","input":"","output":"An adapter MUST NOT import another adapter; only composition-root wires them.","source_path":"CLAUDE.md","corpus_version":"smoke"}
{"instruction":"What extension do relative imports use in scaffolded TS?","input":"","output":"All relative imports use .js extensions (NodeNext).","source_path":"CLAUDE.md","corpus_version":"smoke"}
EOF
  exec python "${PY}" \
    --smoke \
    --data "${tmp}/corpus.jsonl" \
    --out "${tmp}/smoke-adapter.gguf" \
    --device cpu
fi

# Normal path: pass through whatever train-lora.sh sends.
exec python "${PY}" "$@"

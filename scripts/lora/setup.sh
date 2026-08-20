#!/usr/bin/env bash
#
# setup.sh — bootstrap the offline hex LoRA training toolchain (ADR-2606161300).
#
# Creates an isolated uv venv, installs PyTorch against the right CUDA index for this
# box's GPU, installs the trainer deps, and clones llama.cpp for the GGUF converter.
# This is OFFLINE dev tooling — nothing here runs at hex runtime.
#
# RTX 5070 Ti is Blackwell (sm_120) → needs cu128 wheels (torch >= 2.7). Override the
# index with HEX_TORCH_INDEX for a different GPU, or set HEX_TORCH_CPU=1 for a CPU-only
# install (enough for --smoke, not for real bases).

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
readonly SCRIPT_DIR
readonly VENV_DIR="${SCRIPT_DIR}/.venv"
readonly LLAMA_DIR="${SCRIPT_DIR}/llama.cpp"
readonly TORCH_INDEX="${HEX_TORCH_INDEX:-https://download.pytorch.org/whl/cu128}"

if ! command -v uv >/dev/null 2>&1; then
  echo "error: 'uv' not found — install it first (https://docs.astral.sh/uv/)" >&2
  exit 1
fi

echo "==> creating venv at ${VENV_DIR}"
uv venv --python 3.12 "${VENV_DIR}"
# shellcheck disable=SC1091
source "${VENV_DIR}/bin/activate"

echo "==> installing PyTorch"
if [[ "${HEX_TORCH_CPU:-0}" == "1" ]]; then
  uv pip install torch --index-url "https://download.pytorch.org/whl/cpu"
else
  echo "    using CUDA index: ${TORCH_INDEX}"
  uv pip install torch --index-url "${TORCH_INDEX}"
fi

echo "==> installing trainer dependencies"
# bitsandbytes may fail to build on bleeding-edge GPUs; the trainer falls back to bf16,
# so a bitsandbytes failure is non-fatal here.
uv pip install -r "${SCRIPT_DIR}/requirements.txt" || {
  echo "    full install hit an error (likely bitsandbytes) — retrying without it"
  grep -v '^bitsandbytes' "${SCRIPT_DIR}/requirements.txt" >"${SCRIPT_DIR}/.requirements.core.txt"
  uv pip install -r "${SCRIPT_DIR}/.requirements.core.txt"
  rm -f "${SCRIPT_DIR}/.requirements.core.txt"
}

echo "==> fetching llama.cpp (GGUF LoRA converter)"
if [[ ! -d "${LLAMA_DIR}/.git" ]]; then
  git clone --depth 1 https://github.com/ggml-org/llama.cpp "${LLAMA_DIR}"
else
  echo "    already present at ${LLAMA_DIR}"
fi
# The converter needs gguf + numpy; install its requirements into the same venv.
if [[ -f "${LLAMA_DIR}/requirements/requirements-convert_lora_to_gguf.txt" ]]; then
  uv pip install -r "${LLAMA_DIR}/requirements/requirements-convert_lora_to_gguf.txt" || true
fi

echo
echo "✓ toolchain ready."
echo "  Smoke test (CPU, tiny model, no big download):"
echo "    scripts/lora/run.sh smoke"
echo "  Real run, once the GPU is free:"
echo "    HEX_LORA_TRAINER=scripts/lora/run.sh scripts/train-lora.sh hex-boundaries --run --base qwen3:4b"

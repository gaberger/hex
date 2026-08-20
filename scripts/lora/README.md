# hex LoRA training toolchain (offline dev tooling)

ADR-2606161300, Phase 1. **Offline only** — the hex daemon never runs this. It turns a
corpus the daemon produced into a GGUF LoRA adapter the daemon can later *serve*. The
adapter is an idiom prior, never enforcement: removing it must not weaken any gate.

## One-time setup

```bash
scripts/lora/setup.sh          # uv venv + PyTorch (cu128 for Blackwell) + trainer deps + llama.cpp
```

Overrides: `HEX_TORCH_INDEX=<wheel-index>` for a different GPU, `HEX_TORCH_CPU=1` for a
CPU-only install (smoke only).

## Verify the pipeline (no GPU, ~minutes)

```bash
scripts/lora/run.sh smoke      # tiny model on CPU: corpus → train → GGUF
```

## Real run

```bash
# 1. Build the corpus (daemon, needs the corpus endpoint deployed):
hex inference corpus build hex-boundaries

# 2. Train + export GGUF (HEX_LORA_TRAINER defaults to scripts/lora/run.sh):
scripts/train-lora.sh hex-boundaries --run --base qwen3:4b

# 3. Register, then bench-gate (promotes ONLY on measured acceptance lift):
hex inference adapter register --expert hex-boundaries --base qwen3:4b --tier 1 \
  --artifact .hex/adapters/hex-boundaries.gguf --corpus-version <v>
hex inference adapter evaluate hex-boundaries
```

## Notes

- **Hardware (this box):** RTX 5070 Ti, 16GB. `qwen3:4b` fits comfortably. A 32B base
  (`qwen2.5-coder:32b`) is very tight on 16GB — expect to need aggressive 4-bit QLoRA or
  off-box training (`project_qwen_next_hardware_ceiling`). Free the GPU from Ollama
  before a real local run (`hex nexus stop` / stop loaded models).
- **Targets the FFN projection modules** (gate/up/down) at rank 4 / alpha 16 (DMoE
  defaults). Ollama applies the adapter at model load, so the prompt-prefix KV-cache
  stays valid across the ReAct loop.
- **4-bit QLoRA** is used when `bitsandbytes` imports cleanly; otherwise bf16 LoRA
  (the 4B base fits 16GB in bf16, sidestepping bitsandbytes/Blackwell build issues).
- `train_lora.py --hf-base <org/repo>` overrides the Ollama→HF base mapping for models
  not in the built-in table.

#!/usr/bin/env python3
"""hex LoRA idiom-expert trainer (offline dev tooling — ADR-2606161300, Phase 1).

Consumes a corpus produced by `hex inference corpus build <expert>`
(`.hex/corpus/<expert>/corpus.jsonl`) and fine-tunes a small LoRA adapter on a frozen
base model, then exports a GGUF adapter that Ollama can load via an `ADAPTER` directive.

This is NOT runtime code. The hex daemon never calls it. It produces an artifact the
daemon later SERVES, and the adapter is an *idiom prior*, never an enforcement
mechanism (ADR-2606161300 §1) — every correctness gate stays external and unchanged.

Design choices:
  * LoRA rank 4 / alpha 16 (DMoE arXiv:2606.14243 defaults).
  * Targets the FFN projection modules (gate/up/down) — the ADR's "final-layer FFN"
    intent; because Ollama applies the adapter at model *load* (producing a derived
    model), the prompt-prefix KV-cache stays valid regardless, preserving the long
    ReAct loop's efficiency.
  * 4-bit QLoRA when bitsandbytes imports cleanly; otherwise bf16 LoRA (the 4B base
    fits 16GB in bf16, and this sidesteps bitsandbytes/Blackwell build issues).
  * Training never decides promotion — the bench gate does (`hex inference adapter
    evaluate`). This script only writes weights.
"""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
from pathlib import Path

# Ollama model name -> HuggingFace repo for the base weights (training needs HF format,
# not the Ollama GGUF blob). Extend as new tier bases come online. Unknown names require
# --hf-base explicitly so we never train against a silently-wrong base.
OLLAMA_TO_HF = {
    "qwen3:4b": "Qwen/Qwen3-4B",
    "qwen2.5-coder:32b": "Qwen/Qwen2.5-Coder-32B-Instruct",
    "qwen2.5-coder:7b": "Qwen/Qwen2.5-Coder-7B-Instruct",
    "devstral-small-2:24b": "mistralai/Devstral-Small-2507",
}

# Tiny instruct model for `--smoke`: validates the full corpus->train->GGUF path fast,
# on CPU, with no large download.
SMOKE_HF_BASE = "HuggingFaceTB/SmolLM2-135M-Instruct"


def log(msg: str) -> None:
    print(f"[train_lora] {msg}", flush=True)


def die(msg: str, code: int = 1) -> "None":
    print(f"[train_lora] ERROR: {msg}", file=sys.stderr, flush=True)
    sys.exit(code)


def load_corpus(path: Path):
    """Read corpus.jsonl into a list of chat-style records. Skips blank lines."""
    if not path.is_file():
        die(f"corpus not found: {path} — run `hex inference corpus build <expert>` first")
    records = []
    with path.open() as fh:
        for line in fh:
            line = line.strip()
            if not line:
                continue
            obj = json.loads(line)
            instruction = obj.get("instruction", "").strip()
            extra = obj.get("input", "").strip()
            output = obj.get("output", "").strip()
            if not instruction or not output:
                continue
            user = instruction if not extra else f"{instruction}\n\n{extra}"
            records.append({"user": user, "assistant": output})
    if not records:
        die(f"corpus had no usable records: {path}")
    log(f"loaded {len(records)} instruction records from {path}")
    return records


def build_dataset(records, tokenizer):
    """Render records to a single `text` field via the tokenizer chat template."""
    from datasets import Dataset

    def to_text(rec):
        messages = [
            {"role": "user", "content": rec["user"]},
            {"role": "assistant", "content": rec["assistant"]},
        ]
        try:
            return tokenizer.apply_chat_template(messages, tokenize=False)
        except Exception:
            # Fallback for tokenizers without a chat template.
            return f"### Instruction:\n{rec['user']}\n\n### Response:\n{rec['assistant']}\n"

    texts = [to_text(r) for r in records]
    return Dataset.from_dict({"text": texts})


def resolve_hf_base(base: str, hf_base: str | None, smoke: bool) -> str:
    if smoke:
        return SMOKE_HF_BASE
    if hf_base:
        return hf_base
    if base in OLLAMA_TO_HF:
        return OLLAMA_TO_HF[base]
    die(
        f"no HF-repo mapping for base '{base}'. Pass --hf-base <org/repo> "
        f"(known: {', '.join(sorted(OLLAMA_TO_HF))})"
    )


def want_4bit(force_4bit: bool, no_4bit: bool, use_cpu: bool) -> bool:
    if no_4bit or use_cpu:
        return False
    try:
        import bitsandbytes  # noqa: F401
    except Exception as exc:  # pragma: no cover - environment dependent
        log(f"bitsandbytes unavailable ({exc}); using bf16 LoRA instead of 4-bit QLoRA")
        return False
    return True


def train(args) -> Path:
    import torch
    from peft import LoraConfig, get_peft_model
    from transformers import AutoModelForCausalLM, AutoTokenizer
    from trl import SFTConfig, SFTTrainer

    use_cpu = args.device == "cpu" or (args.device == "auto" and not torch.cuda.is_available())
    if use_cpu:
        log("training on CPU (no CUDA or --device cpu) — fine for --smoke, slow for real bases")

    hf_base = resolve_hf_base(args.base, args.hf_base, args.smoke)
    log(f"base (HF): {hf_base}")

    tokenizer = AutoTokenizer.from_pretrained(hf_base, trust_remote_code=True)
    if tokenizer.pad_token is None:
        tokenizer.pad_token = tokenizer.eos_token

    four_bit = want_4bit(args.four_bit, args.no_4bit, use_cpu)
    model_kwargs = {"trust_remote_code": True}
    if four_bit:
        from transformers import BitsAndBytesConfig

        log("loading base in 4-bit (nf4 double-quant) for QLoRA")
        model_kwargs["quantization_config"] = BitsAndBytesConfig(
            load_in_4bit=True,
            bnb_4bit_quant_type="nf4",
            bnb_4bit_use_double_quant=True,
            bnb_4bit_compute_dtype=torch.bfloat16,
        )
        model_kwargs["device_map"] = "auto"
    else:
        model_kwargs["torch_dtype"] = torch.float32 if use_cpu else torch.bfloat16
        if not use_cpu:
            model_kwargs["device_map"] = "auto"

    model = AutoModelForCausalLM.from_pretrained(hf_base, **model_kwargs)
    model.config.use_cache = False

    if four_bit:
        from peft import prepare_model_for_kbit_training

        model = prepare_model_for_kbit_training(model)

    # FFN projection modules ("final-layer FFN" intent, ADR-2606161300/DMoE). Names
    # cover Qwen/Llama/Mistral families; PEFT ignores names a model doesn't have.
    target_modules = ["gate_proj", "up_proj", "down_proj"]
    lora = LoraConfig(
        r=args.rank,
        lora_alpha=args.alpha,
        lora_dropout=0.05,
        bias="none",
        task_type="CAUSAL_LM",
        target_modules=target_modules,
    )
    model = get_peft_model(model, lora)
    model.print_trainable_parameters()

    records = load_corpus(Path(args.data))
    dataset = build_dataset(records, tokenizer)

    out_dir = Path(args.out).resolve()
    work_dir = out_dir.parent / f".{out_dir.stem}-peft"
    work_dir.mkdir(parents=True, exist_ok=True)

    max_len = 512 if args.smoke else args.max_seq_len
    sft_kwargs = dict(
        output_dir=str(work_dir),
        num_train_epochs=1 if args.smoke else args.epochs,
        max_steps=2 if args.smoke else -1,
        per_device_train_batch_size=1,
        gradient_accumulation_steps=1 if args.smoke else 8,
        learning_rate=2e-4,
        logging_steps=1,
        save_strategy="no",
        bf16=not use_cpu and not four_bit,
        report_to=[],
        dataset_text_field="text",
    )
    # trl renamed max_seq_length -> max_length around 1.0; support both.
    try:
        sft = SFTConfig(max_length=max_len, **sft_kwargs)
    except TypeError:
        sft = SFTConfig(max_seq_length=max_len, **sft_kwargs)

    # Newer trl takes the tokenizer as processing_class; older as tokenizer.
    try:
        trainer = SFTTrainer(
            model=model, args=sft, train_dataset=dataset, processing_class=tokenizer
        )
    except TypeError:
        trainer = SFTTrainer(model=model, args=sft, train_dataset=dataset, tokenizer=tokenizer)

    log("starting LoRA fine-tune…")
    trainer.train()

    adapter_dir = work_dir / "adapter"
    model.save_pretrained(str(adapter_dir))
    tokenizer.save_pretrained(str(adapter_dir))
    log(f"PEFT adapter written: {adapter_dir}")

    gguf_path = export_gguf(adapter_dir, hf_base, out_dir, args.llama_cpp)
    return gguf_path


def export_gguf(adapter_dir: Path, hf_base: str, out_path: Path, llama_cpp: str | None) -> Path:
    """Convert a PEFT LoRA adapter to a GGUF the Ollama ADAPTER directive can load."""
    converter = find_converter(llama_cpp)
    if converter is None:
        log(
            "llama.cpp convert_lora_to_gguf.py not found — skipping GGUF export. "
            "Run scripts/lora/setup.sh, or pass --llama-cpp <path>. PEFT adapter is at "
            f"{adapter_dir}"
        )
        return adapter_dir

    out_path.parent.mkdir(parents=True, exist_ok=True)
    # --base-model-id loads base hparams + tensor metadata from the HF hub (cached from
    # training), so no local base-weights directory is required.
    cmd = [
        sys.executable,
        str(converter),
        str(adapter_dir),
        "--base-model-id",
        hf_base,
        "--trust-remote-code",
        "--outfile",
        str(out_path),
        "--outtype",
        "f16",
    ]
    log(f"exporting GGUF: {' '.join(cmd)}")
    proc = subprocess.run(cmd, capture_output=True, text=True)
    if proc.returncode != 0:
        log(proc.stdout)
        log(proc.stderr)
        die(f"GGUF conversion failed (exit {proc.returncode})")
    log(f"GGUF adapter written: {out_path}")
    return out_path


def find_converter(llama_cpp: str | None) -> Path | None:
    candidates = []
    if llama_cpp:
        candidates.append(Path(llama_cpp) / "convert_lora_to_gguf.py")
    here = Path(__file__).resolve().parent
    candidates.append(here / "llama.cpp" / "convert_lora_to_gguf.py")
    env = os.environ.get("LLAMA_CPP_DIR")
    if env:
        candidates.append(Path(env) / "convert_lora_to_gguf.py")
    for c in candidates:
        if c.is_file():
            return c
    return None


def main() -> None:
    p = argparse.ArgumentParser(description="hex LoRA idiom-expert trainer (offline)")
    p.add_argument("--base", default="qwen3:4b", help="Ollama base model name")
    p.add_argument("--hf-base", default=None, help="Override HF repo for the base weights")
    p.add_argument("--data", required=True, help="Path to corpus.jsonl")
    p.add_argument("--out", required=True, help="Output GGUF adapter path")
    p.add_argument("--rank", type=int, default=4)
    p.add_argument("--alpha", type=int, default=16)
    p.add_argument("--epochs", type=int, default=3)
    p.add_argument("--max-seq-len", type=int, default=1024)
    p.add_argument("--device", choices=["auto", "cuda", "cpu"], default="auto")
    p.add_argument("--four-bit", action="store_true", help="Force 4-bit QLoRA")
    p.add_argument("--no-4bit", action="store_true", help="Disable 4-bit; use bf16 LoRA")
    p.add_argument("--llama-cpp", default=None, help="Path to a llama.cpp checkout")
    p.add_argument("--smoke", action="store_true", help="Tiny CPU run to validate the pipeline")
    # --target-modules accepted for compat with train-lora.sh; FFN is always the target.
    p.add_argument("--target-modules", default="final_ffn", help=argparse.SUPPRESS)
    args = p.parse_args()

    out = train(args)
    log(f"DONE → {out}")


if __name__ == "__main__":
    main()

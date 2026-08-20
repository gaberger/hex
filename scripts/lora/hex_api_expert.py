#!/usr/bin/env python3
"""hex-API knowledge expert — applying DMoE-style injection to hex's own API (offline).

The finding so far: LoRA injection is a FACT-teacher, not a rule-enforcer. hex's own API
(its ~50 CLI verbs / MCP tools) is a pure missing-fact for any local model: it's a private
tool, never in training data, and models demonstrably hallucinate non-existent commands
(hence CLAUDE.md's "never recommend commands not in hex --help"). That makes it the ideal
DMoE target — stable, enumerable, authoritative.

Experiment: inject the hex API (from mcp-tools.json) as a knowledge expert into qwen3:4b,
then measure, closed-book, whether the model recalls the REAL command for an intent and
whether it STOPS inventing fake commands — base vs base+expert.

No leakage: per tool, the teacher generates several distinct intent phrasings; we train on
all but one and eval on the held-out phrasing (same fact, unseen wording — the DMoE recall
setup). Metrics: recall (right command), exact (right command core), hallucination rate
(proposed a hex command that doesn't exist).
"""

from __future__ import annotations

import argparse
import json
import os
import re
import urllib.request

OLLAMA_URL = os.environ.get("OLLAMA_URL", "http://localhost:11434")


def log(m: str) -> None:
    print(f"[hex-api] {m}", flush=True)


# ── Load the canonical hex API fact set ────────────────────────────────────────
def core_of(cli: str) -> str:
    """'hex adr search <query>' -> 'adr search' (verb path, args dropped)."""
    s = cli.strip()
    if s.startswith("hex "):
        s = s[4:]
    toks = []
    for t in s.split():
        if t.startswith(("<", "[", "--")):
            break
        toks.append(t)
    return " ".join(toks).strip()


def load_api(path: str):
    tools = json.load(open(path))["tools"]
    facts = []
    for t in tools:
        cli = t.get("cli", "")
        core = core_of(cli)
        if not core:
            continue
        facts.append({"name": t["name"], "cli": cli, "core": core,
                      "desc": t.get("description", ""), "category": t.get("category", "")})
    real_cores = {f["core"] for f in facts}
    real_verbs = {f["core"].split()[0] for f in facts}
    log(f"{len(facts)} hex API commands; {len(real_verbs)} top-level verbs")
    return facts, real_cores, real_verbs


# ── PRAG augmentation: distinct intent phrasings per command ────────────────────
def ollama_chat(model: str, system: str, user: str) -> str:
    body = json.dumps({
        "model": model,
        "messages": [{"role": "system", "content": system}, {"role": "user", "content": user}],
        "stream": False, "think": False, "format": "json",
        "options": {"temperature": 0.6, "num_predict": 512, "num_ctx": 4096},
    }).encode()
    req = urllib.request.Request(f"{OLLAMA_URL}/api/chat", data=body,
                                headers={"content-type": "application/json"})
    with urllib.request.urlopen(req, timeout=300) as r:
        return json.loads(r.read())["message"]["content"]


def make_intents(facts, teacher: str, k: int):
    """Per command, generate k+1 distinct user intents that should map to it."""
    train, evalset = [], []
    for i, f in enumerate(facts, 1):
        user = (
            f"hex CLI command: `{f['cli']}`\n"
            f"What it does: {f['desc']}\n\n"
            f"Write {k + 1} DIFFERENT natural-language ways a developer might ask for this, "
            "WITHOUT naming the command. Vary the wording a lot. Return ONLY JSON: "
            "{\"intents\":[\"...\"]}"
        )
        intents = []
        try:
            txt = ollama_chat(teacher, "Output strict JSON only.", user)
            s, e = txt.find("{"), txt.rfind("}")
            intents = [str(x).strip() for x in json.loads(txt[s:e + 1]).get("intents", []) if str(x).strip()]
        except Exception as exc:  # noqa: BLE001
            log(f"  intent-gen fail [{i}/{len(facts)}] {f['name']}: {str(exc)[:50]}")
        # Fallback so every command still has an eval probe.
        if len(intents) < 2:
            intents = [f"How do I {f['desc'][:60].rstrip('.')}", f"What command does: {f['desc'][:60]}"]
        for q in intents[:k]:
            train.append({"instruction": q, "output": f["cli"], "core": f["core"]})
        evalset.append({"intent": intents[-1], "cli": f["cli"], "core": f["core"]})
        if i % 15 == 0:
            log(f"  intents {i}/{len(facts)} → {len(train)} train records")
    log(f"{len(train)} training records; {len(evalset)} held-out eval probes")
    return train, evalset


# Instruct models need their chat format + a clear system role, else they emit garbage.
SYS = "You are a hex CLI expert. Reply with ONLY the exact hex command for the request, nothing else."


# ── Train (in-process, chat format, COMPLETION-ONLY loss via manual masking) ────
def train_expert(base: str, records, rank: int, epochs: int, max_seq: int):
    import torch
    from datasets import Dataset
    from peft import LoraConfig, get_peft_model
    from transformers import (AutoModelForCausalLM, AutoTokenizer, DataCollatorForSeq2Seq,
                              Trainer, TrainingArguments)

    tok = AutoTokenizer.from_pretrained(base)
    if tok.pad_token is None:
        tok.pad_token = tok.eos_token
    model = AutoModelForCausalLM.from_pretrained(base, torch_dtype=torch.bfloat16, device_map="auto")
    model.config.use_cache = False
    lora = LoraConfig(r=rank, lora_alpha=2 * rank, lora_dropout=0.05, bias="none",
                      task_type="CAUSAL_LM", target_modules=["gate_proj", "up_proj", "down_proj"])
    model = get_peft_model(model, lora)
    model.print_trainable_parameters()

    # Completion-only: mask the prompt (system+user) so loss falls ONLY on the command.
    # This is the fix for the chat-template degeneration ("system system system…") —
    # the model never learns to emit role scaffolding, only the answer.
    rows = []
    for r in records:
        msgs = [{"role": "system", "content": SYS}, {"role": "user", "content": r["instruction"]}]
        prompt = tok.apply_chat_template(msgs, tokenize=False, add_generation_prompt=True)
        full = prompt + r["output"] + tok.eos_token
        p_ids = tok(prompt, add_special_tokens=False)["input_ids"]
        f_ids = tok(full, add_special_tokens=False)["input_ids"][:max_seq]
        labels = [-100] * len(p_ids) + f_ids[len(p_ids):]
        labels = labels[:len(f_ids)]
        rows.append({"input_ids": f_ids, "attention_mask": [1] * len(f_ids), "labels": labels})
    ds = Dataset.from_list(rows)

    args_tr = TrainingArguments(
        output_dir="/tmp/hexapi-train", num_train_epochs=epochs, per_device_train_batch_size=1,
        gradient_accumulation_steps=8, learning_rate=2e-4, logging_steps=20, save_strategy="no",
        bf16=True, report_to=[])
    collator = DataCollatorForSeq2Seq(tok, padding=True, label_pad_token_id=-100)
    Trainer(model=model, args=args_tr, train_dataset=ds, data_collator=collator).train()
    return model, tok


# ── Closed-book command prediction + scoring ───────────────────────────────────
def predict(model, tok, intent: str) -> str:
    import torch
    msgs = [{"role": "system", "content": SYS}, {"role": "user", "content": intent}]
    prompt = tok.apply_chat_template(msgs, tokenize=False, add_generation_prompt=True)
    inp = tok(prompt, return_tensors="pt", add_special_tokens=False).to(model.device)
    with torch.no_grad():
        out = model.generate(**inp, max_new_tokens=24, do_sample=False,
                             repetition_penalty=1.15, eos_token_id=tok.eos_token_id,
                             pad_token_id=tok.pad_token_id)
    txt = tok.decode(out[0][inp["input_ids"].shape[1]:], skip_special_tokens=True)
    return txt.strip().split("\n")[0].strip()


def proposed_core(pred: str) -> str | None:
    """Extract the 'hex <verb...>' core a prediction proposes, if any."""
    m = re.search(r"hex\s+([a-z0-9][a-z0-9 _-]*)", pred.lower())
    if not m:
        return None
    toks = []
    for t in m.group(1).split():
        if t.startswith(("<", "[", "--")):
            break
        toks.append(t)
    return " ".join(toks[:3]).strip() or None


def score(model, tok, evalset, real_cores, real_verbs, label, debug=0):
    recall = exact = proposed = hallucinated = 0
    for i, e in enumerate(evalset):
        pred = predict(model, tok, e["intent"])
        gold = e["core"]
        pc = proposed_core(pred)
        if gold in pred.lower():
            recall += 1
        if pc is not None:
            proposed += 1
            if pc == gold or pc.startswith(gold) or gold.startswith(pc):
                exact += 1
            # Hallucinated = proposed a hex command whose core/verb isn't real.
            ok = pc in real_cores or pc.split()[0] in real_verbs
            if not ok:
                hallucinated += 1
        if i < debug:
            log(f"    intent: {e['intent'][:60]}")
            log(f"    gold='{gold}' pred='{pred[:60]}' core='{pc}'")
    n = len(evalset)
    log(f"{label:<26} recall={recall/n:.2f} exact={exact/n:.2f} "
        f"proposed={proposed/n:.2f} halluc={hallucinated/max(proposed,1):.2f} (n={n})")
    return recall / n, exact / n, hallucinated / max(proposed, 1)


def main() -> None:
    ap = argparse.ArgumentParser(description="hex-API knowledge expert experiment")
    ap.add_argument("--api", default="hex-cli/assets/mcp/mcp-tools.json")
    ap.add_argument("--base", default="Qwen/Qwen2.5-1.5B-Instruct")
    ap.add_argument("--teacher", default="qwen3:4b")
    ap.add_argument("--k", type=int, default=4, help="train intent phrasings per command")
    ap.add_argument("--rank", type=int, default=16)
    ap.add_argument("--epochs", type=int, default=4)
    ap.add_argument("--max-seq", type=int, default=256)
    ap.add_argument("--limit", type=int, default=0, help="cap commands (0=all) for smoke")
    args = ap.parse_args()

    facts, real_cores, real_verbs = load_api(args.api)
    if args.limit:
        facts = facts[: args.limit]
        real_cores = {f["core"] for f in facts}
        real_verbs = {f["core"].split()[0] for f in facts}

    log(f"PRAG intent augmentation via {args.teacher} …")
    train, evalset = make_intents(facts, args.teacher, args.k)

    log(f"training hex-api expert on {args.base} (rank {args.rank}) …")
    model, tok = train_expert(args.base, train, args.rank, args.epochs, args.max_seq)

    dbg = int(os.environ.get("HEXAPI_DEBUG", "0"))
    log("=" * 64)
    log("RESULTS (closed-book: intent -> hex command)")
    a = score(model, tok, evalset, real_cores, real_verbs, "hex-api expert (base+exp)", debug=dbg)
    with model.disable_adapter():
        b = score(model, tok, evalset, real_cores, real_verbs, "Dense (bare base)", debug=dbg)
    log("=" * 64)
    log(f"Recall lift (expert − base):       {a[0] - b[0]:+.2f}")
    log(f"Exact-command lift:                {a[1] - b[1]:+.2f}")
    log(f"Hallucination change (expert−base): {a[2] - b[2]:+.2f}  (lower is better)")


if __name__ == "__main__":
    main()

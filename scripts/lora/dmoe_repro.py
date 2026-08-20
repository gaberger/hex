#!/usr/bin/env python3
"""Scoped reproduction of DMoE's core claim (arXiv:2606.14243), offline tooling.

DMoE = parametric knowledge injection: convert a knowledge corpus into LoRA experts so a
small base can answer factual questions CLOSED-BOOK (no retrieval in the prompt). This
script reproduces the CENTRAL CLAIM — does decoupled LoRA knowledge-injection lift
closed-book QA over the dense base? — on the paper's own model family (Qwen2.5-1.5B-Instruct)
and dataset (HotpotQA), measured with the paper's metrics (EM / F1).

Faithful to the paper:
  * model + dataset + EM/F1 metrics + final-FFN attachment option (DMoE's efficiency choice).
  * PRAG-style augmentation: synthetic QA generated FROM the gold supporting passages
    (the knowledge units), NOT the test questions (no leakage).

Scoped deviations (stated honestly):
  * ONE merged knowledge expert, not one-LoRA-per-document. So we DON'T reproduce the
    BM25 router or the token-uncertainty gate (those need a custom decode loop; Ollama
    can't do them). We test the injection EFFECT, not the full serving architecture.
  * Default attaches FFN LoRA across all layers for capacity (a single merged expert at
    final-FFN-only underfits ~100 passages); --ffn final reproduces the strict attachment.

Conditions reported:
  Dense (closed-book base) | DMoE-lite (closed-book base+expert) | RAG upper-bound (open-book base).
"""

from __future__ import annotations

import argparse
import json
import os
import re
import string
import sys
import urllib.request
from collections import Counter

OLLAMA_URL = os.environ.get("OLLAMA_URL", "http://localhost:11434")


def log(m: str) -> None:
    print(f"[dmoe] {m}", flush=True)


# ── HotpotQA: extract eval QA + knowledge units (gold supporting passages) ──────
def load_hotpot(n: int):
    from datasets import load_dataset

    ds = load_dataset("hotpotqa/hotpot_qa", "distractor", split=f"validation[:{n}]")
    eval_qa = []          # (question, answer, gold_context) — per-question supporting passages
    passages = {}         # title -> full paragraph text (the knowledge units, deduped)
    for ex in ds:
        ctx = ex["context"]
        support_titles = set(ex["supporting_facts"]["title"])
        this_ctx = []
        for title, sents in zip(ctx["title"], ctx["sentences"]):
            if title in support_titles:
                para = " ".join(s.strip() for s in sents).strip()
                passages[title] = para
                this_ctx.append(f"{title}: {para}")
        eval_qa.append((ex["question"], ex["answer"], "\n".join(this_ctx)))
    log(f"{len(eval_qa)} eval questions; {len(passages)} unique knowledge passages")
    return eval_qa, passages


# ── PRAG augmentation: synthetic QA about each passage (no test-question leakage) ──
def ollama_chat(model: str, system: str, user: str) -> str:
    body = json.dumps({
        "model": model,
        "messages": [{"role": "system", "content": system}, {"role": "user", "content": user}],
        "stream": False, "think": False, "format": "json",
        "options": {"temperature": 0.4, "num_predict": 1024, "num_ctx": 4096},
    }).encode()
    req = urllib.request.Request(f"{OLLAMA_URL}/api/chat", data=body,
                                headers={"content-type": "application/json"})
    with urllib.request.urlopen(req, timeout=300) as r:
        return json.loads(r.read())["message"]["content"]


def augment(passages: dict, teacher: str, qa_per: int):
    records = []
    titles = list(passages)
    for i, title in enumerate(titles, 1):
        text = passages[title][:2000]
        # NOTE: we deliberately do NOT train on a "passage dump" record — it teaches the
        # model to emit long passage text instead of short factual answers, which tanks
        # short-answer EM/F1. DMoE-style injection = learn to ANSWER from the knowledge.
        user = (
            f"Passage about '{title}':\n{text}\n\n"
            f"Write exactly {qa_per} factual question/answer pairs answerable SOLELY from this "
            "passage. Answers must be short (a name, date, or phrase). Return ONLY JSON: "
            "{\"qa\":[{\"q\":\"...\",\"a\":\"...\"}]}"
        )
        try:
            txt = ollama_chat(teacher, "Output strict JSON only.", user)
            s, e = txt.find("{"), txt.rfind("}")
            obj = json.loads(txt[s:e + 1])
            for item in obj.get("qa", []):
                q, a = str(item.get("q", "")).strip(), str(item.get("a", "")).strip()
                if q and a:
                    records.append({"instruction": q, "input": "", "output": a})
        except Exception as exc:  # noqa: BLE001
            log(f"  augment fail [{i}/{len(titles)}] {title}: {str(exc)[:60]}")
        if i % 20 == 0:
            log(f"  augmented {i}/{len(titles)} passages → {len(records)} records")
    log(f"augmentation done: {len(records)} training records")
    return records


# ── EM / F1 (SQuAD / HotpotQA standard normalization) ──────────────────────────
def normalize(s: str) -> str:
    s = s.lower()
    s = "".join(ch for ch in s if ch not in set(string.punctuation))
    s = re.sub(r"\b(a|an|the)\b", " ", s)
    return " ".join(s.split())


def em_score(pred: str, gold: str) -> float:
    return float(normalize(pred) == normalize(gold))


def f1_score(pred: str, gold: str) -> float:
    pt, gt = normalize(pred).split(), normalize(gold).split()
    if not pt or not gt:
        return float(pt == gt)
    common = Counter(pt) & Counter(gt)
    same = sum(common.values())
    if same == 0:
        return 0.0
    p, r = same / len(pt), same / len(gt)
    return 2 * p * r / (p + r)


# ── Training (PEFT LoRA, final-FFN option) ─────────────────────────────────────
def train_expert(base_model: str, records, rank: int, epochs: int, ffn: str, max_seq: int):
    import torch
    from datasets import Dataset
    from peft import LoraConfig, get_peft_model
    from transformers import AutoModelForCausalLM, AutoTokenizer
    from trl import SFTConfig, SFTTrainer

    tok = AutoTokenizer.from_pretrained(base_model)
    if tok.pad_token is None:
        tok.pad_token = tok.eos_token
    model = AutoModelForCausalLM.from_pretrained(base_model, torch_dtype=torch.bfloat16, device_map="auto")
    model.config.use_cache = False

    n_layers = model.config.num_hidden_layers
    layers = [n_layers - 1] if ffn == "final" else None  # DMoE attaches to the final-layer FFN
    lora = LoraConfig(
        r=rank, lora_alpha=2 * rank, lora_dropout=0.05, bias="none", task_type="CAUSAL_LM",
        target_modules=["gate_proj", "up_proj", "down_proj"], layers_to_transform=layers,
    )
    model = get_peft_model(model, lora)
    model.print_trainable_parameters()

    # Plain completion format with explicit EOS. Using the chat template here let the
    # LoRA degenerate into looping on role tokens ("system system system…"); a clean
    # QA format that matches the eval prompt fixes that and isolates knowledge injection.
    def to_text(r):
        return f"Question: {r['instruction']}\nAnswer: {r['output']}{tok.eos_token}"

    dataset = Dataset.from_dict({"text": [to_text(r) for r in records]})
    cfg = SFTConfig(output_dir="/tmp/dmoe-train", num_train_epochs=epochs,
                    per_device_train_batch_size=1, gradient_accumulation_steps=8,
                    learning_rate=2e-4, logging_steps=10, save_strategy="no", bf16=True,
                    report_to=[], max_length=max_seq, dataset_text_field="text")
    try:
        trainer = SFTTrainer(model=model, args=cfg, train_dataset=dataset, processing_class=tok)
    except TypeError:
        trainer = SFTTrainer(model=model, args=cfg, train_dataset=dataset, tokenizer=tok)
    trainer.train()
    return model, tok


# ── Closed-book / open-book generation + scoring ───────────────────────────────
def answer(model, tok, question: str, context: str | None) -> str:
    import torch

    # Plain completion prompt — matches the training format so the injected expert and
    # the dense base are evaluated identically.
    if context:
        prompt = f"Context:\n{context}\n\nQuestion: {question}\nAnswer:"
    else:
        prompt = f"Question: {question}\nAnswer:"
    inputs = tok(prompt, return_tensors="pt").to(model.device)
    with torch.no_grad():
        out = model.generate(**inputs, max_new_tokens=40, do_sample=False,
                             repetition_penalty=1.3, eos_token_id=tok.eos_token_id,
                             pad_token_id=tok.pad_token_id)
    text = tok.decode(out[0][inputs["input_ids"].shape[1]:], skip_special_tokens=True)
    return clean_answer(text)


def clean_answer(text: str) -> str:
    """Extract a short answer: drop reasoning/prefixes, keep the first clause."""
    t = text.strip()
    # Take content after an explicit "Answer:" if present.
    m = re.search(r"answer\s*[:\-]\s*(.+)", t, flags=re.IGNORECASE | re.DOTALL)
    if m:
        t = m.group(1).strip()
    t = t.split("\n")[0].strip()
    # Strip common lead-ins.
    t = re.sub(r"^(the answer is|it is|that would be|this is)\s+", "", t, flags=re.IGNORECASE)
    # Keep the first sentence/clause for verbose models.
    t = re.split(r"(?<=[.!?])\s|,\s+because|\s+\(", t)[0]
    return t.strip().strip('"').strip()


def evaluate(model, tok, eval_qa, label: str, open_book: bool = False, debug: int = 0):
    em = f1 = 0.0
    for i, (q, gold, gold_ctx) in enumerate(eval_qa):
        ctx = gold_ctx if open_book else None
        pred = answer(model, tok, q, ctx)
        if i < debug:
            log(f"    Q: {q[:70]}")
            log(f"    gold='{gold}'  pred='{pred[:80]}'")
        em += em_score(pred, gold)
        f1 += f1_score(pred, gold)
    n = len(eval_qa)
    log(f"{label:<34} EM={em / n:.3f}  F1={f1 / n:.3f}  (n={n})")
    return em / n, f1 / n


def main() -> None:
    ap = argparse.ArgumentParser(description="Scoped DMoE knowledge-injection reproduction")
    ap.add_argument("--n", type=int, default=60, help="HotpotQA validation examples")
    ap.add_argument("--base", default="Qwen/Qwen2.5-1.5B-Instruct")
    ap.add_argument("--teacher", default="qwen3:4b", help="Ollama model for PRAG augmentation")
    ap.add_argument("--qa-per", type=int, default=3)
    ap.add_argument("--rank", type=int, default=16)
    ap.add_argument("--epochs", type=int, default=4)
    ap.add_argument("--ffn", choices=["all", "final"], default="all",
                    help="FFN LoRA across all layers (capacity) or final layer only (faithful DMoE)")
    ap.add_argument("--max-seq", type=int, default=1024)
    ap.add_argument("--corpus-out", default="/tmp/dmoe-corpus.jsonl")
    args = ap.parse_args()

    eval_qa, passages = load_hotpot(args.n)

    log(f"PRAG augmentation via {args.teacher} …")
    records = augment(passages, args.teacher, args.qa_per)
    with open(args.corpus_out, "w") as fh:
        for r in records:
            fh.write(json.dumps(r) + "\n")

    log(f"training knowledge expert on {args.base} (ffn={args.ffn}, rank={args.rank}) …")
    model, tok = train_expert(args.base, records, args.rank, args.epochs, args.ffn, args.max_seq)

    log("=" * 64)
    log("RESULTS (HotpotQA closed-book unless noted)")
    dbg = int(os.environ.get("DMOE_DEBUG", "0"))
    # DMoE-lite: expert enabled.
    a_em, a_f1 = evaluate(model, tok, eval_qa, "DMoE-lite (base+expert, closed)", debug=dbg)
    # Dense baseline: disable the adapter → original base.
    with model.disable_adapter():
        b_em, b_f1 = evaluate(model, tok, eval_qa, "Dense (base, closed-book)", debug=dbg)
        # RAG upper bound: base with each question's gold context in the prompt.
        r_em, r_f1 = evaluate(model, tok, eval_qa, "RAG upper-bound (base, open-book)", open_book=True, debug=dbg)
    log("=" * 64)
    log(f"Injection lift (DMoE-lite − Dense):  EM {a_em - b_em:+.3f}   F1 {a_f1 - b_f1:+.3f}")
    log(f"Headroom to RAG upper bound:         EM {r_em - b_em:+.3f}   F1 {r_f1 - b_f1:+.3f}")


if __name__ == "__main__":
    main()

#!/usr/bin/env python3
"""Does injected org knowledge support REASONING, or only RECALL? (offline experiment)

The open question behind "teach a frontier model my business via a local cartridge": when
you bake an org's policies into a model's weights, can it REASON over them (compose facts
into novel conclusions), or does it merely RECALL stated facts? That distinction is the
difference between "understanding" and "lookup".

Design that isolates the two:
  * A synthetic company's ATOMIC policy facts (non-calendar fiscal year, EOY freeze,
    approval thresholds, hierarchy, named CFO) — none guessable by any pretrained model.
  * Train the cartridge ONLY on the atomic facts (recall-level), never on the reasoning
    answers.
  * Eval two question types:
      RECALL    — held-out phrasings of STATED facts.
      REASONING — scenarios whose answers require composing 2-3 facts and are NEVER stated.
  * Three conditions:
      Dense      (bare base, closed-book)
      Cartridge  (base+expert, closed-book)        <- knowledge in WEIGHTS
      Open-book  (base + all facts in context)     <- knowledge in CONTEXT (the control)

Verdict: cartridge-reasoning ≈ open-book-reasoning ≫ base  → injection enables reasoning.
         cartridge-recall high but cartridge-reasoning ≪ open-book → recall only (honest no).
         open-book-reasoning also low → base too weak to test (inconclusive; bigger base).
"""

from __future__ import annotations

import argparse
import json
import os
import urllib.request

OLLAMA_URL = os.environ.get("OLLAMA_URL", "http://localhost:11434")
SYS = "You are an operations assistant for the company. Answer the question concisely and correctly using company policy."

# ── The synthetic company (atomic facts — the injected knowledge) ───────────────
FACTS = [
    "The fiscal year ends on January 31. Q4 covers November, December, and January.",
    "The end-of-year ledger freeze runs from December 15 to January 5 inclusive; no financial transactions are processed during the freeze.",
    "Expense approval thresholds: under $1,000 is auto-approved; $1,000 to $9,999 needs a Manager; $10,000 to $49,999 needs a Director; $50,000 and above needs BOTH the VP and the CFO.",
    "Approval hierarchy is IC, then Manager, then Director, then VP, then CFO. A role may only approve items within its own threshold; a lower role cannot approve a higher-threshold item.",
    "Refunds take 3 business days to process. A refund requested during the freeze is deferred to the first business day after the freeze, which is January 6.",
    "Purchase orders of $50,000 or more also require a 5-business-day review period before they can be approved.",
    "Vendor onboarding takes 10 business days and cannot begin during the end-of-year freeze.",
    "The CFO is Dana Reeves and the VP of Finance is Sam Okoro.",
    "Payroll runs on the last business day of each month; if that day falls inside the freeze, payroll runs on January 6 instead.",
    "Travel reimbursements always require Manager approval, regardless of the amount.",
    "Capital expenditures over $100,000 require board approval at the next quarterly board meeting.",
    "The last two business days of each quarter are 'quarter-close'; only critical transactions are processed then.",
]

# ── RECALL probes: held-out phrasings of STATED facts → (question, [groups]) ─────
# A group is a list of acceptable substrings (any-of); ALL groups must be matched.
RECALL = [
    ("On what date does the company's fiscal year end?", [["january 31", "jan 31"]]),
    ("Who is the company's CFO?", [["dana reeves", "dana"]]),
    ("How many business days does vendor onboarding take?", [["10", "ten"]]),
    ("What approval is needed for an expense between $10,000 and $49,999?", [["director"]]),
    ("How long do refunds take to process?", [["3", "three"]]),
    ("What dates does the end-of-year ledger freeze cover?", [["december 15", "dec 15"], ["january 5", "jan 5"]]),
    ("Who approves a travel reimbursement?", [["manager"]]),
    ("Above what amount does a capital expenditure need board approval?", [["100,000", "100000", "100k"]]),
]

# ── REASONING probes: compose facts; answers NOT stated → (question, [groups]) ───
REASONING = [
    ("A Manager approves a $15,000 software purchase. Is that approval sufficient?",
     [["director"], ["no", "not", "cannot", "insufficient", "need", "require"]]),
    ("It is December 20. A customer requests a refund today. On what date will it be processed?",
     [["january 6", "jan 6"]]),
    ("A $75,000 purchase order is submitted. Who must approve it, and what else is required before approval?",
     [["vp"], ["cfo"], ["review", "5", "five"]]),
    ("Can a new vendor begin onboarding on December 18?",
     [["no", "cannot", "can't", "not", "after", "freeze"], ["january", "jan 6", "freeze"]]),
    ("Payroll's normal run date this month is December 31. On what date will payroll actually run?",
     [["january 6", "jan 6"]]),
    ("A Director approves a $60,000 expense. Is that valid?",
     [["no", "not", "cannot", "insufficient", "need", "require"], ["vp", "cfo"]]),
    ("An employee submits a $500 office-supplies expense. Whose approval is required?",
     [["auto", "automatic", "no approval", "none", "no one", "nobody"]]),
    ("It is January 3. Can a wire transfer be processed today?",
     [["no", "cannot", "can't", "not", "frozen", "freeze"]]),
    ("A $200,000 capital expenditure is proposed. What approval path does it need?",
     [["board"]]),
    ("It is December 16 and a $40,000 expense needs sign-off and processing. Who approves it, and can it be processed today?",
     [["director"], ["no", "not", "cannot", "frozen", "freeze", "after"]]),
]


def log(m: str) -> None:
    print(f"[org] {m}", flush=True)


def ollama_chat(model: str, system: str, user: str) -> str:
    body = json.dumps({
        "model": model,
        "messages": [{"role": "system", "content": system}, {"role": "user", "content": user}],
        "stream": False, "think": False, "format": "json",
        "options": {"temperature": 0.5, "num_predict": 600, "num_ctx": 4096},
    }).encode()
    req = urllib.request.Request(f"{OLLAMA_URL}/api/chat", data=body,
                                headers={"content-type": "application/json"})
    with urllib.request.urlopen(req, timeout=300) as r:
        return json.loads(r.read())["message"]["content"]


def make_training(teacher: str, k: int):
    """PRAG: per atomic fact, k recall-style QA paraphrases (no reasoning answers)."""
    records = []
    for i, fact in enumerate(FACTS, 1):
        # The fact itself as a statement record.
        records.append({"instruction": "State the relevant company policy.", "output": fact})
        user = (f"Company policy fact:\n{fact}\n\nWrite {k} DIFFERENT simple question/answer "
                "pairs that directly ASK about and STATE this fact (recall only, no scenarios). "
                "Short answers. Return ONLY JSON: {\"qa\":[{\"q\":\"...\",\"a\":\"...\"}]}")
        try:
            txt = ollama_chat(teacher, "Output strict JSON only.", user)
            s, e = txt.find("{"), txt.rfind("}")
            for item in json.loads(txt[s:e + 1]).get("qa", []):
                q, a = str(item.get("q", "")).strip(), str(item.get("a", "")).strip()
                if q and a:
                    records.append({"instruction": q, "output": a})
        except Exception as exc:  # noqa: BLE001
            log(f"  augment fail [{i}] {str(exc)[:40]}")
    log(f"{len(records)} training records (facts + recall paraphrases; NO reasoning answers)")
    return records


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
    rows = []
    for r in records:
        msgs = [{"role": "system", "content": SYS}, {"role": "user", "content": r["instruction"]}]
        prompt = tok.apply_chat_template(msgs, tokenize=False, add_generation_prompt=True)
        full = prompt + r["output"] + tok.eos_token
        p_ids = tok(prompt, add_special_tokens=False)["input_ids"]
        f_ids = tok(full, add_special_tokens=False)["input_ids"][:max_seq]
        labels = ([-100] * len(p_ids) + f_ids[len(p_ids):])[:len(f_ids)]
        rows.append({"input_ids": f_ids, "attention_mask": [1] * len(f_ids), "labels": labels})
    ds = Dataset.from_list(rows)
    args_tr = TrainingArguments(output_dir="/tmp/org-train", num_train_epochs=epochs,
                                per_device_train_batch_size=1, gradient_accumulation_steps=8,
                                learning_rate=2e-4, logging_steps=20, save_strategy="no",
                                bf16=True, report_to=[])
    Trainer(model=model, args=args_tr, train_dataset=ds,
            data_collator=DataCollatorForSeq2Seq(tok, padding=True, label_pad_token_id=-100)).train()
    return model, tok


def answer(model, tok, question: str, context: str | None) -> str:
    import torch
    sys_p = SYS if not context else SYS + "\n\nCompany policy:\n" + context
    msgs = [{"role": "system", "content": sys_p}, {"role": "user", "content": question}]
    prompt = tok.apply_chat_template(msgs, tokenize=False, add_generation_prompt=True)
    inp = tok(prompt, return_tensors="pt", add_special_tokens=False).to(model.device)
    with torch.no_grad():
        out = model.generate(**inp, max_new_tokens=90, do_sample=False, repetition_penalty=1.15,
                             eos_token_id=tok.eos_token_id, pad_token_id=tok.pad_token_id)
    return tok.decode(out[0][inp["input_ids"].shape[1]:], skip_special_tokens=True).strip()


def graded(pred: str, groups) -> bool:
    p = pred.lower()
    return all(any(s in p for s in g) for g in groups)


def evaluate(model, tok, probes, label, context=None, debug=0):
    hits = 0
    for i, (q, groups) in enumerate(probes):
        pred = answer(model, tok, q, context)
        ok = graded(pred, groups)
        hits += ok
        if i < debug:
            log(f"    [{'OK ' if ok else 'MISS'}] {q[:58]}")
            log(f"        pred: {pred[:90]}")
    n = len(probes)
    log(f"  {label:<34} {hits}/{n} = {hits/n:.2f}")
    return hits / n


def main() -> None:
    ap = argparse.ArgumentParser(description="org knowledge: recall vs reasoning")
    ap.add_argument("--base", default="Qwen/Qwen2.5-1.5B-Instruct")
    ap.add_argument("--teacher", default="qwen3:4b")
    ap.add_argument("--k", type=int, default=6)
    ap.add_argument("--rank", type=int, default=16)
    ap.add_argument("--epochs", type=int, default=6)
    ap.add_argument("--max-seq", type=int, default=256)
    args = ap.parse_args()

    log(f"PRAG recall-training augmentation via {args.teacher} …")
    records = make_training(args.teacher, args.k)
    log(f"training org cartridge on {args.base} …")
    model, tok = train_expert(args.base, records, args.rank, args.epochs, args.max_seq)

    ctx = "\n".join(f"- {f}" for f in FACTS)
    dbg = int(os.environ.get("ORG_DEBUG", "0"))
    log("=" * 66)
    log("RECALL (held-out phrasings of stated facts):")
    cr = evaluate(model, tok, RECALL, "Cartridge (weights, closed-book)", debug=0)
    with model.disable_adapter():
        br = evaluate(model, tok, RECALL, "Dense (bare base, closed-book)", debug=0)
        orr = evaluate(model, tok, RECALL, "Open-book (facts in context)", context=ctx, debug=0)
    log("REASONING (compose facts; answers never stated):")
    cq = evaluate(model, tok, REASONING, "Cartridge (weights, closed-book)", debug=dbg)
    with model.disable_adapter():
        bq = evaluate(model, tok, REASONING, "Dense (bare base, closed-book)", debug=0)
        oq = evaluate(model, tok, REASONING, "Open-book (facts in context)", context=ctx, debug=dbg)
    log("=" * 66)
    log(f"RECALL    base={br:.2f}  cartridge={cr:.2f}  open-book={orr:.2f}")
    log(f"REASONING base={bq:.2f}  cartridge={cq:.2f}  open-book={oq:.2f}")
    log("-" * 66)
    log(f"Injection RECALL lift (cartridge−base):     {cr-br:+.2f}")
    log(f"Injection REASONING lift (cartridge−base):  {cq-bq:+.2f}")
    log(f"Reasoning gap to context (cartridge−openbk): {cq-oq:+.2f}  (~0 ⇒ weights reason like context)")


if __name__ == "__main__":
    main()

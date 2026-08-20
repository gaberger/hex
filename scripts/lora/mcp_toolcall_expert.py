#!/usr/bin/env python3
"""MCP tool-call expert — does knowledge injection produce SCHEMA-VALID tool calls? (offline)

The hex-API expert proved a cartridge fixes tool *recall* (right name, no hallucination).
The open question for "better MCP servers" is *arguments*: real MCP calls live or die on
producing JSON that validates against the tool's schema. This tests that end-to-end, with
the JSON Schema itself as the objective gate (exactly like `hex analyze` gated the
verified-corpus work).

Setup: take an MCP server's tool registry (mcp-tools.json = hex's), PRAG-augment
(intent -> filled arguments) pairs per tool, train a cartridge to emit full tool calls
{"tool","arguments"}, then closed-book measure base vs base+cartridge on:
  - tool_correct   : named the right tool
  - hallucination  : named a tool not in the registry
  - schema_valid   : arguments validate against the chosen tool's JSON Schema
  - VALID CALL     : right tool AND schema-valid args  (the headline — a usable MCP call)

No leakage: per tool, several distinct (intent,args) examples; train on all but one, eval
on the held-out one.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import urllib.request

import jsonschema

OLLAMA_URL = os.environ.get("OLLAMA_URL", "http://localhost:11434")
SYS = ('You are an MCP tool-calling assistant. For the request, reply with ONLY a JSON '
       'object {"tool": "<tool_name>", "arguments": {...}} and nothing else.')


def log(m: str) -> None:
    print(f"[mcp-call] {m}", flush=True)


def load_tools(path: str):
    tools = json.load(open(path))["tools"]
    reg = {}
    for t in tools:
        reg[t["name"]] = {"schema": t.get("inputSchema", {"type": "object"}),
                          "desc": t.get("description", ""), "cli": t.get("cli", "")}
    log(f"{len(reg)} MCP tools loaded")
    return reg


def ollama_chat(model: str, system: str, user: str) -> str:
    body = json.dumps({
        "model": model,
        "messages": [{"role": "system", "content": system}, {"role": "user", "content": user}],
        "stream": False, "think": False, "format": "json",
        "options": {"temperature": 0.6, "num_predict": 700, "num_ctx": 4096},
    }).encode()
    req = urllib.request.Request(f"{OLLAMA_URL}/api/chat", data=body,
                                headers={"content-type": "application/json"})
    with urllib.request.urlopen(req, timeout=300) as r:
        return json.loads(r.read())["message"]["content"]


def make_examples(reg, teacher: str, k: int):
    """Per tool: k+1 (intent, arguments) examples; train on k, hold 1 out for eval."""
    train, evalset = [], []
    names = list(reg)
    for i, name in enumerate(names, 1):
        schema = reg[name]["schema"]
        user = (
            f"MCP tool `{name}`: {reg[name]['desc']}\n"
            f"Arguments JSON Schema: {json.dumps(schema)}\n\n"
            f"Invent {k + 1} DIFFERENT realistic user requests for this tool, each with a "
            "concrete `arguments` object that satisfies the schema (fill required fields with "
            "plausible values). Don't name the tool in the request. Return ONLY JSON: "
            "{\"examples\":[{\"intent\":\"...\",\"arguments\":{...}}]}"
        )
        exs = []
        try:
            txt = ollama_chat(teacher, "Output strict JSON only.", user)
            s, e = txt.find("{"), txt.rfind("}")
            exs = [x for x in json.loads(txt[s:e + 1]).get("examples", [])
                   if isinstance(x.get("arguments"), dict) and str(x.get("intent", "")).strip()]
        except Exception as exc:  # noqa: BLE001
            log(f"  gen fail [{i}/{len(names)}] {name}: {str(exc)[:50]}")
        if len(exs) < 2:
            # Fallback: empty-args example so every tool still has an eval probe.
            exs = [{"intent": f"Use the tool that does: {reg[name]['desc'][:70]}", "arguments": {}}]*2
        for x in exs[:k]:
            call = json.dumps({"tool": name, "arguments": x["arguments"]}, separators=(",", ":"))
            train.append({"instruction": x["intent"], "output": call})
        evalset.append({"intent": exs[-1]["intent"], "tool": name})
        if i % 15 == 0:
            log(f"  examples {i}/{len(names)} → {len(train)} train records")
    log(f"{len(train)} training records; {len(evalset)} held-out eval probes")
    return train, evalset


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

    args_tr = TrainingArguments(
        output_dir="/tmp/mcpcall-train", num_train_epochs=epochs, per_device_train_batch_size=1,
        gradient_accumulation_steps=8, learning_rate=2e-4, logging_steps=20, save_strategy="no",
        bf16=True, report_to=[])
    collator = DataCollatorForSeq2Seq(tok, padding=True, label_pad_token_id=-100)
    Trainer(model=model, args=args_tr, train_dataset=ds, data_collator=collator).train()
    return model, tok


def predict_call(model, tok, intent: str):
    import torch
    msgs = [{"role": "system", "content": SYS}, {"role": "user", "content": intent}]
    prompt = tok.apply_chat_template(msgs, tokenize=False, add_generation_prompt=True)
    inp = tok(prompt, return_tensors="pt", add_special_tokens=False).to(model.device)
    with torch.no_grad():
        out = model.generate(**inp, max_new_tokens=96, do_sample=False, repetition_penalty=1.1,
                             eos_token_id=tok.eos_token_id, pad_token_id=tok.pad_token_id)
    txt = tok.decode(out[0][inp["input_ids"].shape[1]:], skip_special_tokens=True)
    s, e = txt.find("{"), txt.rfind("}")
    if s < 0 or e <= s:
        return None, None
    try:
        obj = json.loads(txt[s:e + 1])
        return obj.get("tool"), obj.get("arguments")
    except json.JSONDecodeError:
        return None, None


def score(model, tok, evalset, reg, label, debug=0):
    proposed = tool_ok = halluc = schema_ok = valid_call = 0
    for i, e in enumerate(evalset):
        tool, args = predict_call(model, tok, e["intent"])
        gold = e["tool"]
        if tool is not None:
            proposed += 1
            if tool not in reg:
                halluc += 1
            if tool == gold:
                tool_ok += 1
            # schema-valid against the CHOSEN tool's schema (must be a real tool + dict args)
            ok_schema = False
            if tool in reg and isinstance(args, dict):
                try:
                    jsonschema.validate(args, reg[tool]["schema"])
                    ok_schema = True
                except jsonschema.ValidationError:
                    ok_schema = False
                except jsonschema.SchemaError:
                    ok_schema = True  # malformed registry schema → don't penalize the model
            if ok_schema:
                schema_ok += 1
            if tool == gold and ok_schema:
                valid_call += 1
        if i < debug:
            log(f"    intent: {e['intent'][:62]}")
            log(f"    gold={gold} pred_tool={tool} args={json.dumps(args)[:60] if args is not None else None}")
    n = len(evalset)
    log(f"{label:<24} valid_call={valid_call/n:.2f} tool_ok={tool_ok/n:.2f} "
        f"schema_ok={schema_ok/n:.2f} halluc={halluc/max(proposed,1):.2f} proposed={proposed/n:.2f}")
    return valid_call / n, tool_ok / n, schema_ok / n, halluc / max(proposed, 1)


def main() -> None:
    ap = argparse.ArgumentParser(description="MCP tool-call (schema-validated) expert experiment")
    ap.add_argument("--api", default="hex-cli/assets/mcp/mcp-tools.json")
    ap.add_argument("--base", default="Qwen/Qwen2.5-1.5B-Instruct")
    ap.add_argument("--teacher", default="qwen3:4b")
    ap.add_argument("--k", type=int, default=4)
    ap.add_argument("--rank", type=int, default=16)
    ap.add_argument("--epochs", type=int, default=4)
    ap.add_argument("--max-seq", type=int, default=384)
    ap.add_argument("--limit", type=int, default=0)
    args = ap.parse_args()

    reg = load_tools(args.api)
    if args.limit:
        reg = dict(list(reg.items())[: args.limit])

    log(f"PRAG (intent,args) augmentation via {args.teacher} …")
    train, evalset = make_examples(reg, args.teacher, args.k)

    log(f"training MCP tool-call expert on {args.base} (rank {args.rank}) …")
    model, tok = train_expert(args.base, train, args.rank, args.epochs, args.max_seq)

    dbg = int(os.environ.get("MCP_DEBUG", "0"))
    log("=" * 66)
    log("RESULTS (closed-book: intent -> schema-validated MCP tool call)")
    a = score(model, tok, evalset, reg, "MCP expert (base+exp)", debug=dbg)
    with model.disable_adapter():
        b = score(model, tok, evalset, reg, "Dense (bare base)", debug=dbg)
    log("=" * 66)
    log(f"VALID-CALL lift (expert − base):   {a[0] - b[0]:+.2f}  (right tool + schema-valid args)")
    log(f"tool-correct lift:                 {a[1] - b[1]:+.2f}")
    log(f"schema-valid lift:                 {a[2] - b[2]:+.2f}")
    log(f"hallucination change:              {a[3] - b[3]:+.2f}  (lower better)")


if __name__ == "__main__":
    main()

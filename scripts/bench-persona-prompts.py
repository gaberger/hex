#!/usr/bin/env python3
"""
Persona-prompt benchmark — PROMPT-VARIANT ITERATION TOOL.

For day-to-day "is this model better than the current default" checks, use
`hex inference bench <model>` instead — the same persona/chat, persona/commit,
and persona/drafter checks are baked into the CLI and run against the
PRODUCTION prompts (hex-cli/src/commands/inference.rs::bench_persona_*).

This script remains for the use case `hex inference bench` doesn't cover:
testing alternate PROMPT STRATEGIES (v1/v2/v3 in this file) against multiple
models in one shot, so we can A/B which system-prompt shape wins per task
without rebuilding the Rust binary. When a v(n) prompt wins decisively,
port its contents into persona_prompt() / conversational_prompt() in
hex-nexus/src/orchestration/org_responder.rs and the v2-equivalent gets
codified.

Runs a fixed test suite against multiple local models (and optionally
OpenRouter frontier models) to measure how well each handles the THREE
real shapes the responder/drafter ask for:

  1. chat-mode      — operator asks a brief, persona answers prose
  2. commit-mode    — operator gives directive, persona emits Confirm:
  3. drafter-mode   — drafter writes file content per a commitment

Each response is scored automatically on:
  - format-adherence  (does it match the expected shape?)
  - brevity           (under the word cap?)
  - grounding         (cites real ADR IDs / file paths?)
  - latency           (wall-clock seconds)
  - cost              (output tokens)

Output: a per-model scorecard so we can see which model + which prompt
combination actually works for OUR task — and re-run as new models drop.

Usage:
  python3 scripts/bench-persona-prompts.py                 # local models, default prompts
  python3 scripts/bench-persona-prompts.py --prompt v2     # try alternate prompt strategy
  python3 scripts/bench-persona-prompts.py --model qwen3:8b nemotron-mini  # custom set
"""

import argparse
import json
import re
import sys
import time
import urllib.error
import urllib.request

OLLAMA = "http://localhost:11434"

# ── Test cases ───────────────────────────────────────────────────────────────

# Each case has:
#   id          — short identifier
#   mode        — chat / commit / drafter
#   system      — system prompt (we'll swap variants in --prompt vN)
#   user        — the operator's message
#   eval        — a function (content, latency_s, output_tokens) → dict of scores
#                 each score is 0..1

REAL_ADR_IDS = ["ADR-001", "ADR-027", "ADR-2026-05-08-2500", "ADR-2026-05-08-2300",
                "ADR-2026-04-11-2000", "ADR-013", "ADR-014", "ADR-025"]
REAL_PATHS = ["docs/specs/", "hex-nexus/src/", "hex-cli/src/", "spacetime-modules/",
              "hex-nexus/assets/src/", "hex-cli/assets/agents/"]

def grounded(text):
    """Count distinct real ADR ids + file paths in the response."""
    n = 0
    lower = text.lower()
    for aid in REAL_ADR_IDS:
        if aid.lower() in lower: n += 1
    for p in REAL_PATHS:
        if p.lower() in lower: n += 1
    return n

def has_meta_reasoning(text):
    """Detect rambling 'I will reply with…' meta-output."""
    lower = text.lower()
    BAD = [
        "we are in",
        "the user is asking",
        "let me recall",
        "let me think",
        "i need to recall",
        "i'll respond with",
        "i will respond",
        "first, i note",
        "key points from",
        "looking at the",
    ]
    return any(b in lower[:400] for b in BAD)

def word_count(text):
    return len(re.findall(r"\b[\w-]+\b", text))


def eval_chat(content, latency_s, out_tokens, target_words=60):
    """Status reply: brief, grounded, no meta-reasoning."""
    wc = word_count(content)
    g = grounded(content)
    return {
        "brevity":       1.0 if wc <= target_words else max(0.0, 1.0 - (wc - target_words) / target_words),
        "grounding":     min(1.0, g / 2.0),  # 2+ refs is full mark
        "no_meta":       0.0 if has_meta_reasoning(content) else 1.0,
        "non_empty":     1.0 if content.strip() else 0.0,
        "latency_score": max(0.0, 1.0 - latency_s / 30.0),  # 30s = floor
        "raw_words":     wc,
        "raw_grounded":  g,
    }


def eval_commit(content, latency_s, out_tokens):
    """Confirm: line. EXACTLY one line starting with 'Confirm:' OR the word 'Silent'."""
    stripped = content.strip()
    first_line = stripped.split("\n", 1)[0].strip()
    is_confirm = first_line.lower().startswith("confirm:")
    is_silent = stripped.lower() in ("silent", "silent.")
    return {
        "format":        1.0 if (is_confirm or is_silent) else 0.0,
        "single_line":   1.0 if "\n" not in stripped or stripped.count("\n") <= 1 else 0.0,
        "grounding":     min(1.0, grounded(content) / 1.0),
        "no_meta":       0.0 if has_meta_reasoning(content) else 1.0,
        "non_empty":     1.0 if content.strip() else 0.0,
        "latency_score": max(0.0, 1.0 - latency_s / 30.0),
        "raw_first":     first_line[:80],
    }


def eval_drafter(content, latency_s, out_tokens):
    """File-body output. Should not preamble; should match the brief."""
    stripped = content.strip()
    starts_clean = not any(stripped.lower().startswith(p) for p in
                          ["okay", "sure", "here", "i'll", "below", "let me", "i will"])
    return {
        "no_preamble":   1.0 if starts_clean else 0.0,
        "in_size":       1.0 if 5 <= len(stripped) <= 4096 else 0.5,
        "no_meta":       0.0 if has_meta_reasoning(content) else 1.0,
        "non_empty":     1.0 if content.strip() else 0.0,
        "latency_score": max(0.0, 1.0 - latency_s / 30.0),
        "raw_bytes":     len(stripped),
    }


# ── Prompt variants ──────────────────────────────────────────────────────────

PROMPTS = {
    "v1": {
        # Current production prompts (paraphrased / matched to org_responder.rs)
        "chat":   "You are the {role}. Answer the operator's question in 2-3 sentences. "
                  "Cite a real ADR id (e.g. ADR-2026-05-08-2500) or repo file path "
                  "(e.g. docs/specs/X.md). Do not narrate what you are about to say.",
        "commit": "You are the {role}. Reply with EXACTLY ONE line in the form:\n"
                  "Confirm: I ({role}) will <action> by <deadline> — success: <artifact>\n"
                  "OR the single word: Silent\n"
                  "No prose. No preamble. Nothing else.",
        "drafter":"You are the {role}. Write the body of `{path}` per the operator's request below. "
                  "Output ONLY the file contents. No preamble. No code fences.",
    },
    "v2": {
        # Tighter: explicit anti-preamble + example output
        "chat":   "You are {role}. Brief format. Direct answer only.\n"
                  "Example output:\n"
                  "Shipped: docs/specs/X.md. In flight: ADR-2026-05-08-2500. Concern: persona rollback rate.\n\n"
                  "Now answer the operator. No 'we are', 'the user', 'let me' — just the answer.",
        "commit": "You are {role}. Reply with one line.\n"
                  "Examples:\n"
                  "Confirm: I (cto) will write docs/specs/foo.md by EOD — success: docs/specs/foo.md\n"
                  "Confirm: I (cpo) will add ADR-2026-05-12-0900 by EOW — success: docs/adrs/ADR-2026-05-12-0900-X.md\n"
                  "Silent\n"
                  "No other format. Start with 'Confirm:' or 'Silent'.",
        "drafter":"You are {role}. Output the file body for `{path}` now. "
                  "First character of output is first character of the file. "
                  "No 'Okay', no 'Sure', no 'Here is', no code fences.",
    },
    "v3": {
        # Anti-thinking: explicit forbidden phrases + small constraint set
        "chat":   "You are {role}. Reply in <=3 sentences. Cite >=1 ADR id or file path. "
                  "Banned phrases: 'we are', 'the user', 'let me think', 'i'll respond', "
                  "'first,', 'looking at'. Just answer.",
        "commit": "OUTPUT FORMAT: `Confirm: I ({role}) will X by Y — success: PATH` OR `Silent`. "
                  "Any other output is invalid. Begin response with C or S now.",
        "drafter":"Write `{path}` body. First char must be valid content. No preamble of any kind.",
    },
}


# ── Test suite ───────────────────────────────────────────────────────────────

def build_cases(prompt_v):
    p = PROMPTS[prompt_v]
    return [
        # Chat: brief status with grounding required
        ("chat_status_cto", "chat",
         p["chat"].format(role="CTO"),
         "Status: shipped today, in flight, top concern. Cite ADR ids."),
        ("chat_status_coo", "chat",
         p["chat"].format(role="COO"),
         "What is the operator's top blocker right now?"),
        # Commit: directive eliciting Confirm: line
        ("commit_write_spec", "commit",
         p["commit"].format(role="cto"),
         "Write docs/specs/sample-output-X.md by EOD. Reply with Confirm line."),
        ("commit_review_adr", "commit",
         p["commit"].format(role="ciso"),
         "Review ADR-013 security claims by tomorrow. Reply with Confirm line."),
        # Drafter: file body for explicit content
        ("drafter_one_line", "drafter",
         p["drafter"].format(role="cto", path="docs/specs/X.md"),
         "The file should contain only one line: Hello from the pipeline."),
        ("drafter_short_spec", "drafter",
         p["drafter"].format(role="cpo", path="docs/specs/Y.md"),
         "Write a 3-bullet spec for adding a Status badge to the dashboard nav."),
    ]


# ── Runner ───────────────────────────────────────────────────────────────────

def call_ollama(model, system, user, num_predict=200, timeout_s=60):
    body = {
        "model": model,
        "messages": [
            {"role": "system", "content": system},
            {"role": "user", "content": user},
        ],
        "stream": False,
        "think": False,
        "options": {"num_predict": num_predict},
    }
    req = urllib.request.Request(
        f"{OLLAMA}/api/chat",
        data=json.dumps(body).encode(),
        headers={"Content-Type": "application/json"},
    )
    t0 = time.time()
    try:
        with urllib.request.urlopen(req, timeout=timeout_s) as resp:
            data = json.loads(resp.read().decode())
        elapsed = time.time() - t0
        content = data.get("message", {}).get("content", "")
        # Strip <think> blocks (qwen3 etc.)
        content = re.sub(r"<think>.*?</think>", "", content, flags=re.DOTALL).strip()
        return {
            "ok": True,
            "content": content,
            "latency_s": elapsed,
            "out_tokens": data.get("eval_count", 0),
        }
    except urllib.error.URLError as e:
        return {"ok": False, "error": str(e), "latency_s": time.time() - t0}
    except Exception as e:
        return {"ok": False, "error": str(e), "latency_s": time.time() - t0}


def run(models, prompt_v):
    cases = build_cases(prompt_v)
    results = {m: {} for m in models}
    eval_fns = {"chat": eval_chat, "commit": eval_commit, "drafter": eval_drafter}

    for model in models:
        print(f"\n── {model} ──", flush=True)
        for case_id, mode, system, user in cases:
            r = call_ollama(model, system, user)
            if not r["ok"]:
                print(f"  ✗ {case_id:24} ERROR: {r.get('error','?')[:60]}")
                results[model][case_id] = {"ok": False, "score": 0.0}
                continue
            scores = eval_fns[mode](r["content"], r["latency_s"], r["out_tokens"])
            # Composite: mean of normalized scores (exclude raw_*)
            score_keys = [k for k in scores if not k.startswith("raw_")]
            composite = sum(scores[k] for k in score_keys) / len(score_keys)
            results[model][case_id] = {
                "ok": True,
                "score": composite,
                "details": scores,
                "latency_s": r["latency_s"],
                "out_tokens": r["out_tokens"],
                "snippet": r["content"][:80].replace("\n", " "),
            }
            print(f"  {('✓' if composite > 0.6 else '◐' if composite > 0.3 else '✗')} {case_id:24} "
                  f"score={composite:.2f}  lat={r['latency_s']:.1f}s  tok={r['out_tokens']}  "
                  f"{r['content'][:60].replace(chr(10),' ')!r}")
    return results, cases


def report(results, cases):
    print("\n\n=== SUMMARY ===\n")
    cols = [c[0] for c in cases]
    header = f"{'model':30} " + " ".join(f"{c[:6]:>7}" for c in cols) + "   AVG"
    print(header)
    print("-" * len(header))
    for m, runs in results.items():
        cells = []
        scores = []
        for c in cols:
            s = runs.get(c, {}).get("score", 0.0)
            scores.append(s)
            cells.append(f"{s:.2f}")
        avg = sum(scores) / len(scores) if scores else 0
        row = f"{m:30} " + " ".join(f"{x:>7}" for x in cells) + f"   {avg:.2f}"
        print(row)
    print()
    # Best model per case
    print("Best model per task:")
    for c in cols:
        best = max(results.keys(), key=lambda m: results[m].get(c, {}).get("score", 0))
        s = results[best].get(c, {}).get("score", 0)
        print(f"  {c:24} → {best:30} {s:.2f}")


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--model", nargs="+", default=None,
                        help="models to bench; default = all detected local models")
    parser.add_argument("--prompt", default="v1", choices=list(PROMPTS.keys()),
                        help="prompt variant to test")
    parser.add_argument("--json", action="store_true", help="emit JSON to stdout")
    args = parser.parse_args()

    if args.model:
        models = args.model
    else:
        # Detect via Ollama
        try:
            with urllib.request.urlopen(f"{OLLAMA}/api/tags", timeout=5) as r:
                data = json.loads(r.read().decode())
            all_models = [m["name"] for m in data.get("models", [])]
            # Filter: skip the very large or cloud-only models for default run
            models = [m for m in all_models
                      if not m.endswith(":cloud") and "32b" not in m and "27b" not in m]
            models = sorted(set(models))[:6]  # cap at 6 for time
        except Exception as e:
            print(f"can't list models: {e}"); sys.exit(1)

    print(f"prompt variant: {args.prompt}")
    print(f"models: {models}")
    results, cases = run(models, args.prompt)
    if args.json:
        print(json.dumps({"prompt": args.prompt, "models": models, "results": results}, indent=2))
    else:
        report(results, cases)


if __name__ == "__main__":
    main()

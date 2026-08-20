#!/usr/bin/env python3
"""Phase 1 dev-utility for docs/benchmarks/dspark-vllm-speculative-decoding-test-plan.md.

Runs the same T1 fixture prompts against Ollama and vLLM (OpenAI-compatible
endpoints) and reports tokens/sec, TTFT-proxy, and accepted length so the
three configs (Ollama baseline / vLLM plain / vLLM + speculative decoding)
can be compared. Throwaway measurement tool per the plan's "Execution
mechanics" section - not a permanent addition.
"""
import argparse
import json
import statistics
import time
import urllib.request

FIXTURES = {
    "t1-fix-add": (
        "You are given the following Rust file `bench-sandbox/src/lib.rs`:\n\n"
        "```rust\npub fn add(a: i64, b: i64) -> i64 { a - b }\n```\n\n"
        "Task: The function `add` in the target file returns `a - b`, which is wrong. "
        "Fix it so it returns the sum `a + b`. Change nothing else.\n\n"
        "Respond with ONLY the corrected full file contents in a rust code block, nothing else."
    ),
    "t1-add-derive": (
        "You are given the following Rust file `bench-sandbox/src/lib.rs`:\n\n"
        "```rust\n#[derive(Clone)]\npub struct Widget { pub id: u32 }\n```\n\n"
        "Task: The struct `Widget` in the target file derives Clone. Add `Debug` to its "
        "derive list so the struct is `#[derive(Clone, Debug)]`. Change nothing else.\n\n"
        "Respond with ONLY the corrected full file contents in a rust code block, nothing else."
    ),
}


def run_ollama(host, model, prompt):
    req = urllib.request.Request(
        f"{host}/api/generate",
        data=json.dumps({"model": model, "prompt": prompt, "stream": False, "think": False}).encode(),
        headers={"Content-Type": "application/json"},
    )
    t0 = time.time()
    with urllib.request.urlopen(req, timeout=300) as resp:
        d = json.load(resp)
    wall = time.time() - t0
    eval_count = d.get("eval_count", 0)
    eval_ns = d.get("eval_duration", 1)
    prompt_eval_ns = d.get("prompt_eval_duration", 0)
    return {
        "wall_s": wall,
        "tokens": eval_count,
        "tok_per_s": eval_count / (eval_ns / 1e9) if eval_ns else 0.0,
        "ttft_proxy_s": prompt_eval_ns / 1e9,
    }


def run_openai_compat(host, model, prompt):
    body = {
        "model": model,
        "messages": [{"role": "user", "content": prompt}],
        "stream": False,
        "max_tokens": 1024,
    }
    req = urllib.request.Request(
        f"{host}/v1/chat/completions",
        data=json.dumps(body).encode(),
        headers={"Content-Type": "application/json"},
    )
    t0 = time.time()
    with urllib.request.urlopen(req, timeout=300) as resp:
        d = json.load(resp)
    wall = time.time() - t0
    usage = d.get("usage", {})
    tokens = usage.get("completion_tokens", 0)
    return {
        "wall_s": wall,
        "tokens": tokens,
        "tok_per_s": tokens / wall if wall else 0.0,
        "ttft_proxy_s": None,  # non-streaming; TTFT needs --stream, out of scope for this pass
    }


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--backend", choices=["ollama", "vllm"], required=True)
    ap.add_argument("--host", default="http://localhost:11434")
    ap.add_argument("--model", required=True)
    ap.add_argument("--reps", type=int, default=3)
    ap.add_argument("--label", default="")
    args = ap.parse_args()

    runner = run_ollama if args.backend == "ollama" else run_openai_compat

    print(f"=== backend={args.backend} model={args.model} label={args.label!r} reps={args.reps} ===")
    for fid, prompt in FIXTURES.items():
        results = [runner(args.host, args.model, prompt) for _ in range(args.reps)]
        tok_s = [r["tok_per_s"] for r in results]
        wall = [r["wall_s"] for r in results]
        toks = [r["tokens"] for r in results]
        print(
            f"{fid:16s} tok/s mean={statistics.mean(tok_s):7.2f} "
            f"wall_s mean={statistics.mean(wall):6.2f} "
            f"tokens mean={statistics.mean(toks):6.1f} "
            f"(n={args.reps})"
        )


if __name__ == "__main__":
    main()

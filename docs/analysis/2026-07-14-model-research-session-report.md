# 2026-07-14 — Model Research Session Report

**Date:** 2026-07-13 → 2026-07-14
**Scope:** Three external models investigated (swellweb/reame, Ornith-1.0-35B, GLM-5.2) plus every hex-internal
bug found along the way. One feature shipped as a direct result: ADR-2607140850 (Continuous HuggingFace
Model Researcher).
**Hardware:** RTX 5070 Ti, 16GB VRAM / 30GB system RAM (this box) — every hardware-feasibility verdict below
is measured against this specific ceiling, not a generic "modern GPU."
**Relates to:** ADR-2607140850, `docs/benchmarks/dspark-vllm-speculative-decoding-test-plan.md`,
`docs/benchmarks/llamacpp-batched-best-of-n-test-plan.md`

---

## Executive summary

Investigated three models with three different outcomes — one partially-useful open-source tool, one
misdiagnosed local model, and one hardware-infeasible frontier release — and in the process of testing
all three, found **four real, previously-undocumented bugs in hex's own inference/tooling stack**, two of
which (missing `max_tokens`/`num_ctx` forwarding, and a 22-hour-wedged `bun test` hang silently stalling
two unrelated workplans) are load-bearing enough to have been quietly degrading hex's local-model
benchmarks and autonomous pipeline all along. One new capability shipped end-to-end through hex's real
feature pipeline (ADR → specs → workplan → tiered-inference codegen → reconcile → complete) as a direct
consequence: a daily HuggingFace model-discovery tick that won't repeat this session's manual, one-model-
at-a-time investigation pattern.

## 1. swellweb/reame — partially real, mostly not portable

CPU-first llama.cpp-based inference server, MIT licensed. Tested directly (built from source, ran
locally).

**Real and working:**
- **The Conclave (`--best-of N`)** — batches N candidates sharing one prefill, genuinely parallel (5
  candidates in ~11s wall-clock at 1356% CPU, not 5× sequential). Honestly reports `consensus=1/5` (no
  false confidence) when a small model's samples disagree.
- Its OpenAI-compatible `--serve` API and speculative-decoding stats endpoint work as documented.

**Claimed but broken:**
- The headline feature — persistent disk KV-cache ("never compute the same thing twice") — never fired
  in any test. Sent a byte-identical 2,332-token-prefix request twice: **8.57s cold vs. 8.70s "cached"**,
  zero cache files ever written, even after finding and setting an undocumented `cache.block_tokens` key.
  The binary contains a real `PrefixCache`/`DiskCacheStore` implementation — just not engaging on the
  `--serve` HTTP path as tested.
- n-gram/prompt-lookup speculative decoding: even on a best-case verbatim-repetition task, no net
  wall-clock win (`overall_speed_tps` ≤ `target_speed_tps` despite 40-43% draft acceptance). This
  independently corroborates the DSpark benchmark's own T1 finding (below) under a different
  engine (llama.cpp vs. vLLM) and different hardware regime (CPU vs. GPU) — about as settled as this
  gets without an engine-level change.

**Verdict:** not adopted as-is. But testing it surfaced that llama.cpp's *upstream* server already has
the primitive reame's Conclave wraps — `n_cmpl` (aliased to OpenAI's `n`), confirmed via hex's own
vendored llama.cpp source (`scripts/lora/llama.cpp/tools/server/`) — plus working longest-common-prefix
slot caching. That finding is now `docs/benchmarks/llamacpp-batched-best-of-n-test-plan.md`: a Phase-0-
gated test plan (not yet run) for whether a new `LlamaCppServerAdapter` could give hex's own
`ScaffoldedDispatch` (which runs T2/T2.5 best-of-N as fully sequential Ollama calls today — confirmed via
`scaffolding.rs:195-231`) a real batching win.

## 2. Ornith-1.0-35B — re-diagnosed, not what the original benchmark said

Prior project memory (`project_ornith_react_bench`, 2026-06-30) recorded: "Ornith is a fast/one-shot DUD
— needs its multi-step self-scaffold loop," based on a `hex bench agentic` run scoring it 58% (react) /
0% (fast) against `devstral-small-2:24b`'s 66%.

**That explanation was wrong.** Checked the actual `deepreinforce-ai/Ornith-1` repo: "self-scaffolding" is
a *training-time* RL technique (the model jointly learns its own reasoning scaffold during RL, baked into
weights as `<think>` CoT) — not a special inference-time harness requirement. The GGUF's embedded chat
template is correct and complete (verified via `ollama show --template`: proper Qwen3-style `<think>` +
nested-XML `<tool_call>` handling).

**Live-tested against the real `t2-humanize-duration` fixture** (production-shaped system prompt +
seeded file content, matching `direct_react.rs`'s actual construction): Ornith's reasoning is genuinely
good — correctly derived the algorithm and verified all 8 required test cases by hand, unprompted
self-verification visible in the raw transcript. The actual failure mode is **protocol adherence, not
capability**: even with an unconstrained token budget, it never emits hex's expected tool-call format —
it narrates ("Let's produce the response... Done. Proceeding...") and outputs a plain ` ```rust ` doc-style
fence instead of the ` ```json ` block hex's system prompt explicitly demands.

**Tried fixing this via constrained decoding — made it worse, not better.** Forcing a JSON-schema output
via Ollama's `format` field suppressed `<think>` reasoning entirely (0 reasoning tokens vs. hundreds
unconstrained) and collapsed answer quality to schema-valid garbage (`{"tool":"cargo_check","args":
{"mode":"append","new_string":""}}` — wrong tool, blank payload). Forcing syntax from token 0 traded
"right answer, wrong format" for "right format, no answer." Ollama's `/api/generate` + `format` is
all-or-nothing; a real fix needs "think freely, then constrain only the final segment," which needs
llama.cpp-server-native GBNF applied post-`</think>` — not available through Ollama's HTTP API.

**Net verdict:** keep `devstral-small-2:24b` as the incumbent react_model (unchanged), but for a different
and more specific reason than originally recorded, and the original benchmark itself was confounded by
bug #1 below. Ornith's demonstrated strength (correct reasoning, clean code fences) is a better match for
hex's *non-agentic* compile-gate best-of-N path (`tier_models.t2`/`t2.5`) than the ReAct tool-use path it
was actually tested in — untested, but a concrete, cheap next experiment if picked back up.

## 3. GLM-5.2 — hardware-infeasible, confirmed by two independent sources

744B total / 40B active MoE, MIT licensed, genuinely frontier-competitive (per Z.ai: 1pt behind Claude
Opus 4.8 on FrontierSWE). Local inference ruled out, not assumed:

- Unsloth's own quality-optimized dynamic quant: smallest usable is ~241GB (2-bit).
- A community effort *specifically* built for minimum-spec/retired-datacenter hardware ("e-waste
  edition"): smallest variant is **226.9 GiB**, and its target hardware is 8×32GB *retired datacenter*
  GPUs — not a home box.

Two independently-motivated quantization efforts (one for quality, one explicitly for minimum spec) land
in the same ~227-241GB floor. This box has 46GB total (16GB GPU + 30GB RAM) — roughly 5× short, not a
"push it and it's slow" gap. No smaller "Air"-style sibling exists for this model family. Confirmed via
OpenRouter that cloud access exists ($0.93/$3 per Mtok) but a paid test was explicitly declined this
session (local-only policy).

## 4. Bugs found in hex's own stack (the more consequential half of this session)

Four separate, real gaps surfaced purely as a side effect of testing the above — none were what was being
looked for.

1. **`OllamaInferenceAdapter` never forwards `max_tokens`/`temperature`/`num_ctx` to Ollama at all**
   (`hex-nexus/src/adapters/inference/ollama.rs` — confirmed via full-file grep: zero matches for any of
   these terms or an `options` object). Every local-model call runs at Ollama's raw default context
   window. Proved directly: an explicit `num_predict=4096` request still cut off at 1684 tokens —
   *context* exhaustion, not the output cap, because nothing raises `num_ctx`. Verbose reasoning models
   hit this fastest, but it silently affects every model on this adapter, including the incumbent
   `devstral-small-2:24b`. Likely the same root cause, more precisely diagnosed, as an earlier memory note
   about Nemotron scoring 0 due to "max_tokens cap truncates before final answer."

2. **The GBNF `grammar` field is dead code end-to-end.** `InferenceRequest.grammar`'s doc comment claims
   it reaches Ollama via `options.grammar`; the adapter actually sends it top-level. Doesn't matter either
   way — tested both placements against live Ollama with a trivial grammar and adequate budget: **both
   silently ignored**, full free-form prose came back regardless. Ollama's real supported mechanism is the
   `format` JSON-Schema field (confirmed working), not raw GBNF text — a bigger fix than "move it into
   options."

3. **`hex plan create`'s workplan output doesn't match what `hex plan execute` actually needs.** `create`
   emits a `steps[]`-based shape that `hex plan lint` happily accepts, but `execute`/nexus's
   `/api/workplan/execute` require the `phases[]`-based canonical schema from `hex plan schema`. Feeding
   `create`'s raw output straight to `execute` silently dispatches **zero tasks**, no error. A second
   internal consumer (`workplan_conductor`) expects the *old* `steps[]` shape and logs `drive failed:
   workplan missing steps[]` every ~60s against a perfectly valid `phases[]` file — two subsystems, two
   incompatible schemas, same file.

4. **A repo-wide `bun test` hang inside `hex dev validate`, silently wedging two unrelated workplans for
   over a day.** `/proc/<pid>/fd` showed thousands of file descriptors open into
   `target/debug/incremental/*` — bun's test discovery/coverage appears to walk the entire repo tree
   including Rust's build cache rather than respecting `.gitignore`. Found three stuck instances: one just
   launched, and **two owned by the live hex-nexus daemon itself, wedged since 2026-07-13 14:02 and 14:18
   — over 22 hours** — almost certainly why `feat-hex-lora-idiom-phase01` (0/9) and
   `feat-autonomous-model-bench` (0/8) had been stalled the entire session with recurring
   `workplan_conductor: stall escalation` warnings. Killed all three as a live mitigation; root cause
   (bun's file-discovery scope) not yet fixed.

Full detail on 1-3: `project_ollama_adapter_missing_options.md`, `project_hex_plan_tooling_gaps.md` (agent
memory). Bug 4 is also in `project_hex_plan_tooling_gaps.md`.

## 5. What shipped as a result

**ADR-2607140850 (Continuous HuggingFace Model Researcher) — Completed.** Rather than keep investigating
one model at a time by hand, scoped and shipped a daily `sched_service` tick that discovers new HF
releases, judges real hardware feasibility (this session's own GLM-5.2/Ornith lessons hardcoded as seed
test cases), auto-benches anything that's actually local-feasible with no approval gate, and only ever
*surfaces* (never auto-tests) anything needing a paid API — preserving the exact boundary this session
held for GLM-5.2. All four phases were generated by hex's own tiered inference dispatch end-to-end (not
hand-written), verified against real compile/test gates, and committed:
`7603e972`/`b56f0218`/`96e2ebf0`/`496a585b`.

## 6. Open threads

- `docs/benchmarks/llamacpp-batched-best-of-n-test-plan.md` — scoped, not yet run. Needs a `llama-server`
  binary pulled and a real Phase 0/1 measurement before any ADR gets written.
- Ornith as a `tier_models.t2`/`t2.5` compile-gate candidate — plausible per §2, untested.
- The `bun test`/`target/` hang's actual root cause (likely a missing ignore-pattern in whatever drives
  bun's file discovery) — not fixed, only mitigated by killing the stuck processes.
- hex-nexus's own binary is currently stale relative to this session's commits (touched
  `sched_service.rs`/`coordination/inbox.rs`) — needs a rebuild + restart before the new tick is live,
  blocked on the same `hex dev validate` hang above.

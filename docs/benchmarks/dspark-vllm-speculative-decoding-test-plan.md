# Test plan: does a vLLM + speculative-decoding swap actually help a hex tier?

**Status:** Phase 0–2 executed 2026-07-01 on `qwen3:4b` (T1) — FAIL, for both vLLM (even
quantization-matched) and llama.cpp-native speculative decoding. But the *same day*, on a bigger
stand-in model (`qwen2.5-coder:14b`), llama.cpp-native speculative decoding was a **clear +27% win**
over Ollama with 100% draft acceptance. **Net conclusion: speculative decoding is real and worth
having on hex's local tiers, but not on T1 — only on models expensive enough per step to amortize it.**
See [Results](#results--2026-07-01), [Follow-up 2](#follow-up-2-llamacpp-native-speculative-decoding-same-day),
and [Follow-up 3](#follow-up-3-biggerslower-model-same-day--the-first-real-win) below.
**Related:** DSpark paper analysis (`deepseek-ai/DeepSpec`, see project memory `project_dspark_speculative_decoding`);
[ADR-2606071734](../adrs/ADR-2606071734-agentic-inference-benchmark-suite.md) (agentic benchmark suite —
this reuses its fixture corpus and axis-6 "throughput economics" metrics rather than inventing a
parallel harness).

## Objective

DSpark's own eval set includes Qwen3-4B and Gemma4-12B, which are hex's live **T1** (`qwen3:4b`) and
**T2** (`gemma4-12b`) tier models — but hex serves them via Ollama, which doesn't expose draft-model
speculative decoding. vLLM is already a first-class hex inference provider type and does support it.
This plan answers one question before any ADR gets written: **does swapping a hex tier to vLLM
(+ speculative decoding) produce a real win on this actual box**, or does it net out to nothing (or
regress) once real hardware constraints and hex's actual workload shape are accounted for.

**Hypothesis to falsify (H1):** vLLM + n-gram/prompt-lookup speculative decoding reduces p50
per-request latency by ≥15% for the pilot tier vs. the current Ollama baseline, with zero regression
in oracle-verified correctness and no VRAM overflow.

## Scope

- **Phase 1 pilot (cleared by Phase 0):** `qwen3:4b` (T1) — highest request volume, comfortable VRAM
  headroom, well-supported vLLM architecture, and one of DSpark's own directly evaluated target
  models, so results are sanity-checkable against their published accepted-length numbers.
- **Blocked pending a quantization check:** `devstral-small-2:24b` (T2.5, the configured
  `react_model`, most latency-sensitive tier) — Phase 0 found only ~800MB of VRAM headroom above its
  15GB Q4_K_M weight footprint on this 16GB card, and vLLM's GGUF/`mistral3` support is unconfirmed.
  Needs its own check (does vLLM load this GGUF cleanly, or source an AWQ requant) before any Phase 1
  run.
- **Deferred:** `gemma4-12b` (T2, 7.1GB, reasonable headroom) — same protocol, after the T1 pilot.
- **Out of scope this round:** training a bespoke DSpark Markov head (the expensive Phase-4 option
  from the earlier research — only worth revisiting if off-the-shelf n-gram decoding under-delivers
  but drafting itself clearly helps).

## Phase 0 — Hardware feasibility gate (before running anything)

**Run 2026-07-01.** Measured, not assumed:

- GPU: RTX 5070 Ti, 16303 MiB total, 15826 MiB free with nothing loaded. System RAM: 30GB (24GB
  available). vLLM not installed. Ollama 0.30.4 running.
- `devstral-small-2:24b` (Mistral3, 24B params, Q4_K_M GGUF) is **15GB on disk for weights alone** —
  leaves ~800MB of headroom on this card. vLLM's PagedAttention allocator wants a large up-front
  reservation for KV cache blocks, and vLLM's mature/fast quantization paths are AWQ/GPTQ/FP8/safetensors
  — GGUF loading is newer and less proven, and `mistral3` GGUF support in vLLM is unconfirmed.
- `qwen3:4b` (2.5GB, Q4_K_M) has generous headroom regardless of backend, and Qwen3 is a first-class,
  well-supported vLLM architecture.

**Decision:** reorder the pilot. `devstral-small-2:24b` (T2.5) is **blocked pending a separate
quantization check** (confirm vLLM loads this GGUF cleanly, or source/build an AWQ requant that fits
comfortably) — do not spend Phase 1 effort on it yet. `qwen3:4b` (T1) is cleared to pilot first: safe
fit, well-supported architecture, and it's one of DSpark's own directly-evaluated target models so
results are sanity-checkable against their published numbers. `gemma4-12b` (T2, 7.1GB) sits in
between — reasonable headroom, revisit after the T1 pilot.

## Phase 1 — Raw serving-layer benchmark (isolate engine swap from drafting effect)

Three configs, same model, same prompts — isolating "is vLLM itself faster here" from "does drafting
help":

| Config | What it isolates |
|---|---|
| (a) Ollama (current) | baseline |
| (b) vLLM, no speculative decoding | engine-swap effect alone |
| (c) vLLM + speculative decoding (ngram/prompt-lookup) | drafting effect alone |

(Confirm exact vLLM flags against the installed version — recent vLLM uses a `--speculative-config`
JSON block with `method: "ngram"` for self-speculation, no separate draft model needed; older versions
use `--speculative-model [ngram]`.)

**Prompts:** reuse the `instruction` field from the existing tier-tagged fixtures
(`docs/benchmarks/fixtures/t1-*.json`, `t2-*.json`, `t25-*.json`) — they're already curated,
tier-matched, and represent hex's actual workload shape. No need to invent synthetic prompts.

**Metrics per config** (mirrors both DSpark's own Table 1 / Fig. 7-8 methodology and hex's existing
axis-6 "throughput economics"):
- time-to-first-token (TTFT)
- tokens/sec per request, single-stream
- accepted length τ (config c only; τ=1 implicitly for a/b)
- peak VRAM resident
- aggregate tokens/sec at 2 and 4 concurrent requests (rough proxy for real HexFlo swarm parallelism)

## Phase 2 — Correctness / regression gate

Speculative decoding is lossless by construction (same target-model output distribution) — verify
that empirically rather than trusting the paper. For each fixture, run its already-defined oracle
(`cargo_test`/`cargo_check` command in the fixture JSON) against the edit each config actually
produced. Expect identical pass/fail across (a)/(b)/(c) on every fixture. Any divergence is a
correctness bug in the vLLM speculative path and is a stop-ship signal for that config, independent of
whatever Phase 1 latency numbers say.

## Phase 3 — Concurrent-load curve (conditional)

Only run this if Phase 1 shows a real per-request win. Sweep concurrency (1, 2, 4, 8 simulated agent
requests) and plot aggregate throughput vs. per-request tok/s for configs (a) and (c) — a mini version
of DSpark's Fig. 7/8. This is where the known GPU arena-contention issue
(`project_nexus_arena_spin`) is most likely to resurface; watch for it specifically.

## Decision gate

- **PASS** (≥15% real per-request latency win, zero correctness regression, fits hardware) → draft the
  ADR, and extend `hex config inference bench --agentic` — the runner ADR-2606071734 already flags as
  follow-up work — with a `--backend ollama|vllm` dimension for ongoing regression tracking. Roll out
  to the pilot tier.
- **MARGINAL** (5–15%, or hardware-tight) → record findings in project memory, shelve. Re-visit if
  hardware changes or a trained EAGLE checkpoint surfaces for these exact models.
- **FAIL** (<5%, any correctness regression, or a Phase 0 hardware blocker) → shelve, no ADR.

## Results — 2026-07-01

Ran Phases 1–2 on `qwen3:4b` (T1) via `scripts/benchmark-vllm-specdecode.py`, vLLM 0.24.0,
`Qwen/Qwen3-4B` (bf16, from HF hub — no AWQ/GGUF build attempted), `--enforce-eager`,
`--gpu-memory-utilization 0.85`, `--speculative-config '{"method":"ngram","num_speculative_tokens":5,
"prompt_lookup_max":4,"prompt_lookup_min":2}'`. Both T1 fixtures (`t1-fix-add`, `t1-add-derive`), n=3
each.

| Config | tok/s (mean, both fixtures) | vs. Ollama baseline |
|---|---|---|
| (a) Ollama, Q4_K_M GGUF | **~236** | — |
| (b) vLLM, bf16, no spec-decode | **~85** | **-64%** (2.8x slower) |
| (c) vLLM, bf16, + ngram spec-decode | **~97** | **-59%** (2.4x slower) |

vLLM's own SpecDecoding metrics for config (c): mean acceptance length 2.0–2.31, avg draft acceptance
rate 20–26%, per-position acceptance decaying from ~0.32–0.47 (position 1) to ~0.10–0.20 (position 5)
— empirically reproducing the exact suffix-decay curve DSpark's paper describes for untrained/naive
parallel drafters on non-repetitive text. n-gram/prompt-lookup drafting is free but weak; it's nowhere
near DSpark's own trained-Markov-head acceptance rates (70–90%+ per position in their paper).

**Phase 2 correctness spot-check:** pulled a fresh completion from config (c) for `t1-fix-add`,
materialized it into the fixture's sandbox, ran the oracle (`cargo test --manifest-path
bench-sandbox/Cargo.toml --test add`) — **3/3 tests passed.** No correctness regression observed.

**Root cause of the FAIL:** it isn't about drafting — it's quantization format, not serving engine.
Ollama's Q4_K_M (4-bit) GGUF via llama.cpp is dramatically more memory-bandwidth-efficient for
single-stream (batch=1) decode than vLLM's unquantized bf16 weights, and single-stream decode is
exactly the regime T1 tasks run in. Speculative decoding's ~13% gain inside vLLM (85→97 tok/s)
doesn't come close to closing a 2.8x quantization-format gap. **This result does not mean speculative
decoding doesn't work — it means this specific comparison (bf16 vLLM vs. Q4_K_M Ollama) confounds two
variables at once.**

### Follow-up: quantization-matched re-test (AWQ), same day

Ran the official `Qwen/Qwen3-4B-AWQ` (4-bit, 411K HF downloads) through the same protocol to isolate
whether the earlier FAIL was purely a quantization-format artifact:

| Config | tok/s (mean, both fixtures) | vs. Ollama baseline |
|---|---|---|
| (a) Ollama, Q4_K_M GGUF | ~236 | — |
| (b) vLLM, bf16, no spec-decode | ~85 | -64% |
| (c) vLLM, bf16, + ngram spec-decode | ~97 | -59% |
| (d) vLLM, **AWQ 4-bit**, no spec-decode | **~145** | **-39%** |
| (e) vLLM, **AWQ 4-bit**, + ngram spec-decode | **~151** | **-36%** |

Matching quantization (bf16 → AWQ 4-bit) closed most of the gap (85→145 tok/s, +70%), confirming
quantization format was indeed a major factor. But a **~36% deficit remains even quantization-matched**
— spec-decode's acceptance metrics were consistent with the earlier run (mean acceptance length 2.13,
22.7% draft acceptance) and added only ~4% on top of AWQ-plain, not enough to close the rest.
Correctness re-confirmed (3/3 oracle pass on a fresh AWQ+spec-decode completion).

**Revised root cause — two factors, not fully separable on this hardware:**

1. **Regime mismatch.** vLLM's architecture (PagedAttention, continuous batching) is built for
   maximizing *aggregate* throughput across many concurrent requests — exactly the production scenario
   DSpark's 60-85% win was measured in (DeepSeek-V4 under hundreds of concurrent requests).
   llama.cpp/Ollama's kernels are historically hand-tuned for the opposite: single-stream, low-batch,
   low-latency decode on one consumer GPU — hex's actual local-tier workload.
2. **Blackwell kernel maturity, checked against the logs, not assumed.** This box's RTX 5070 Ti is
   compute capability `(12, 0)` (sm_120) — a very new architecture. vLLM selected **FlashAttention v2**
   as its attention backend (not v3/FlashInfer), and one spec-decode run logged a live
   `Triton kernel JIT compilation during inference: _compute_slot_mapping_kernel. This causes a
   latency spike` warning — i.e. vLLM was compiling kernels on-the-fly mid-benchmark because it lacked
   a pre-tuned config for this GPU/shape. The AWQ path *did* get the fast Marlin fused kernel
   (`Using MarlinLinearKernel for AutoAWQMarlinLinearMethod`), so that part wasn't degraded — which is
   consistent with AWQ closing most (not all) of the gap. Ollama/llama.cpp runs on this same GPU
   without the analogous slowdown, because its CUDA kernels are simpler and don't depend on the same
   stack of specialized libraries (FlashAttention/FlashInfer/Marlin/Triton-autotune) that are still
   catching up to sm_120.

**Can't fully separate these two factors without a second, older GPU (Ampere/Hopper/Ada) to A/B
against — not available on this box.** The honest read: some real fraction of the 36% gap is
GPU-generation-specific kernel immaturity that may shrink as vLLM/FlashAttention/FlashInfer mature for
Blackwell, and some fraction is the structural single-stream-vs-throughput mismatch that won't go away
regardless of GPU. **DSpark's own headline result (measured on DeepSeek's own hyperscale hardware and
serving stack) doesn't transfer to this box's deployment shape today** — but "today, on this GPU" is
doing real work in that sentence; revisit if this GPU generation's vLLM kernel support matures, or if
testing ever happens on non-Blackwell hardware.

**Decision:** FAIL against the plan's gate, confirmed even with the confound removed. Shelve —
no ADR, no vLLM migration for local tiers on this hardware/workload shape, **for now**. Devstral (T2.5)
and Gemma4-12B (T2) pilots not attempted — the AWQ re-test already answered the load-bearing question
well enough (vLLM's disadvantage here is regime-mismatch-plus-Blackwell-kernel-immaturity, not
model-specific), so repeating the protocol on larger models would likely just re-confirm the same gap
at higher VRAM cost. Re-open this if: vLLM/FlashAttention/FlashInfer ship mature sm_120 (Blackwell)
kernel support, or this ever gets tested on Ampere/Hopper/Ada hardware where the numbers could look
meaningfully different.

**What's actually worth trying instead, if T1/T2.5 latency becomes a real pain point:** llama.cpp
itself (which Ollama wraps) supports draft-model speculative decoding natively via `llama-server
--model-draft`, keeping the fast Q4_K_M kernels that are winning here while adding drafting on top —
Ollama doesn't expose this flag today. That would need a new "raw llama.cpp" provider path in hex, not
a vLLM swap, and is a separate, not-yet-scoped piece of work.

## Follow-up 2: llama.cpp-native speculative decoding, same day

The most promising untried lead from the vLLM results above: llama.cpp itself (which Ollama wraps)
supports `--model-draft` speculative decoding with a *real* draft model (not just n-gram matching),
which should keep the fast Q4_K_M kernels that won every round against vLLM. Tested directly.

**Setup:** No CUDA toolkit on this box (driver only, no `nvcc`), and llama.cpp ships no prebuilt Linux
CUDA binaries — only Windows CUDA and several Linux non-CUDA backends. Used the prebuilt
`llama-b9857-bin-ubuntu-vulkan-x64` release instead: the NVIDIA Vulkan ICD was already present
(`libnvidia-gl-580`), so this GPU gets real Vulkan compute acceleration without needing a source build.
Target and draft weights were used directly from Ollama's own blob store (`/usr/share/ollama/.ollama/models/blobs/sha256-*`,
world-readable, valid GGUF headers) — no re-download needed for the target; pulled `qwen3:0.6b` for a
same-family, same-tokenizer draft model (only ~520MB, mostly already deduped/cached by Ollama).

| Config | tok/s (mean, both fixtures) | vs. Ollama baseline | draft acceptance |
|---|---|---|---|
| Ollama, Q4_K_M (CUDA backend) | ~236 | — | — |
| llama.cpp, Q4_K_M (**Vulkan**), plain, no draft | ~192 | -19% | — |
| llama.cpp, Q4_K_M (Vulkan), + real draft, `n-max=8` | ~133 | -44% | 54-71% (mean len ~3.0-4.0) |
| llama.cpp, Q4_K_M (Vulkan), + real draft, `n-max=4` (tuned) | ~177 | -25% | 62-83% (mean len ~2.7-3.1) |

Two findings, both real:

1. **Same kernels, different GPU backend, and the backend gap alone (~19%) is much smaller than
   vLLM's best result (~36%, quantization-matched).** This is consistent with the earlier hypothesis:
   llama.cpp's simpler CUDA kernels aren't available prebuilt for Linux, but even its more
   general-purpose Vulkan path lands far closer to Ollama's native CUDA path than vLLM's
   FlashAttention/PagedAttention stack does on this same sm_120 GPU.
2. **Speculative decoding itself was a wash-to-regression here, despite a legitimately good draft.**
   Acceptance rates of 54-83% and mean accepted lengths of ~3-4 tokens/round are *far* better than
   vLLM's 20-26% n-gram matching — this is what a properly-matched draft model looks like, not a
   config error. Untuned (`n-max=8`) it actively regressed throughput (-44%, worse than plain);
   tuning the draft length down to roughly match the observed acceptance length (`n-max=4`) recovered
   most of the loss but still landed at parity-or-slightly-worse (-25%) vs. plain llama.cpp with no
   draft at all. Oracle correctness held (3/3 pass, checked on the tuned config).

**Why a well-accepted draft still doesn't win here:** qwen3:4b is already hex's *smallest, fastest*
tier (T1) — each autoregressive step is already cheap on this GPU (~200 tok/s plain). Speculative
decoding's win comes from amortizing an expensive target-verification pass over several draft tokens;
that only pays off when a single autoregressive step is expensive enough that batched verification is
meaningfully cheaper per accepted token than the sequential draft-generation + dispatch/sync overhead
between two separate models. For a target this small and already this fast, that overhead is
comparable to the savings — there isn't much slack left to amortize into.

**Implication for hex, if this ever gets revisited:** the T1 tier (small, already-fast models) is
close to the *worst* candidate for speculative decoding of any kind. The more promising target is a
genuinely large/slow tier — tested below.

## Follow-up 3: bigger/slower model, same day — the first real win

Tried to test this directly on hex's actual T2 (`gemma4-12b`) and T2.5 (`devstral-small-2:24b`)
tiers and hit real obstacles on *both*, worth recording rather than hiding:

- `gemma4-12b` has **no smaller same-family Gemma4 sibling** in Ollama's library (`gemma4:2b`,
  `gemma4:1b` don't exist) — no draft-model candidate available.
- `devstral-small-2:24b` has **no VRAM headroom** for a second resident model (15GB weights on a
  16GB card, per Phase 0) — even if a smaller Devstral sibling existed.
- `qwen2.5-coder:32b` (the LoRA `augment_model`) is **19GB on disk — doesn't fit fully in 16GB VRAM
  at all**, so it wasn't a clean target either (would need partial CPU offload, confounding the
  comparison with PCIe transfer cost, unrelated to the speculative-decoding question).

Substituted `qwen2.5-coder:14b` (9GB, comfortable fit, already in the local fleet) as target with
`qwen2.5-coder:1.5b` as draft (same family/tokenizer) — not one of hex's three named tiers, but a
genuine bigger/slower-per-step stand-in that isolates the same variable.

| Config | tok/s (mean, both fixtures) | vs. Ollama baseline |
|---|---|---|
| Ollama, Q4_K_M (CUDA) | ~86 | — |
| llama.cpp, Q4_K_M (Vulkan), plain | ~72 | -16% (consistent with the qwen3:4b backend gap) |
| llama.cpp, Q4_K_M (Vulkan), **+ real draft** | **~109** | **+27%** |

**First clear win of the whole investigation.** Draft acceptance was **100%** (24/24, 19/19 accepted
across sampled requests, mean accepted length ~4.8-5.0, i.e. hitting the `n-max=4`+bonus-token
ceiling essentially every round) — the 1.5B draft predicted the 14B target's output almost perfectly
on this low-entropy, near-deterministic mechanical-fix task. +27% over Ollama's own baseline, +51%
over llama.cpp-plain-no-draft. Oracle correctness held (3/3 pass).

**This confirms the earlier hypothesis directly:** speculative decoding needs an expensive-enough
target step to amortize against. qwen3:4b (T1) didn't have that slack; qwen2.5-coder:14b does, and the
win was immediate and large even without much tuning. **Reframed recommendation:** if hex wants to
chase this, the win is real and available today for any tier at or above ~14B params *if* a
same-family smaller sibling model exists and fits alongside the target in VRAM — which rules out
`devstral-small-2:24b` (no headroom) and `gemma4-12b` (no sibling) as they stand today, but not, e.g.,
adding a `qwen2.5-coder:1.5b`-drafted `qwen2.5-coder:14b` tier, or sourcing a smaller Devstral/Gemma4
build later. Correctness-wise this is genuinely low-risk (rejection sampling is lossless by
construction, and it held in every test run this session, T1 and mid-size alike).

## Execution mechanics

- Phases 1–2 run via a single-purpose `scripts/benchmark-vllm-specdecode.sh` — a dev-utility, allowed
  under the no-runtime-scripts rule's explicit exception (`scripts/benchmark-*.sh`). This is a
  throwaway measurement tool, not a permanent addition.
- If the decision gate passes, the durable version of this capability is the `--backend` flag on the
  real `hex config inference bench --agentic` runner — not the script kept around long-term.
- Everything here is local and read-only against this box's own GPU; no shared state, no destructive
  actions. The only real-world side effect is installing vLLM as a new local dependency.

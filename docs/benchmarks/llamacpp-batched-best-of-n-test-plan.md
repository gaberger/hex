# Test plan: does a llama.cpp-server batched best-of-N adapter beat sequential Ollama calls?

**Status:** NOT YET RUN. Source-level feasibility check only (2026-07-13) — confirmed the batching
primitive exists in llama.cpp's server today; no benchmark executed yet. This plan gates the ADR the
same way [dspark-vllm-speculative-decoding-test-plan.md](dspark-vllm-speculative-decoding-test-plan.md)
gated the speculative-decoding ADR: measure first, decide after.

**Related:** [dspark-vllm-speculative-decoding-test-plan.md](dspark-vllm-speculative-decoding-test-plan.md)
(Follow-up 2/3 already flagged "a new raw llama.cpp provider path in hex" as unscoped future work — this
plan picks up that thread for the batching angle specifically, not speculative decoding);
ADR-2026-04-12-0202 (Scaffolded Dispatch — Best-of-N + compile gate, the mechanism being targeted);
project memory `project_dspark_speculative_decoding` (established that GGUF/Q4_K_M on this box's CUDA
path beats vLLM's bf16/AWQ for single-stream decode — a fact this plan must not re-litigate, only extend
to the batched-request regime, which is different).

## Objective

`ScaffoldedDispatch::dispatch()` (`hex-nexus/src/orchestration/scaffolding.rs:195-231`) generates N
completions per task for T2 (N=3) and T2.5 (N=5) tiers, checks each against a compile gate, and keeps
the first pass. Today this is a plain `for i in 0..n { self.inference.complete(...).await? }` loop —
fully sequential, each candidate re-pays the full prompt prefill against Ollama's `/api/generate`.
`workplan_executor.rs`'s own comments clock the resulting T2.5 latency at 250–400s end-to-end.

Source inspection of hex's already-vendored llama.cpp checkout (`scripts/lora/llama.cpp/tools/server/`)
found the server has a built-in mechanism for exactly this: `n_cmpl` (aliased to OpenAI's `n`,
`server-task.h:61`, `server-task.cpp:273`) generates N completions from **one shared prompt prefill** —
"when the main prompt is processed, activate all child tasks too" (`server-context.cpp:3374,3849-3850`).
Gated by `n_cmpl <= n_parallel` (slot count), enforced at `server-task.cpp:617-618`. Separately, slot
selection already does longest-common-prefix matching across resident slots
(`slot_prompt_similarity` / `get_available_slot`, `server-context.cpp:1389-1426`) — an in-memory,
per-process version of the persistent-prefix-cache idea that swellweb/reame's disk implementation failed
to deliver in testing (see project conversation, 2026-07-13).

**This plan answers one question before any ADR gets written:** does routing T2/T2.5 best-of-N through
a llama.cpp-server adapter using `n_cmpl` actually beat sequential Ollama calls on this box, once the
CUDA-vs-Vulkan backend tax (already measured at ~16-19% in the DSpark doc, Follow-ups 2/3) is priced in —
or does that tax eat the batching win, the same way it ate the speculative-decoding win on T1.

**Hypothesis to falsify (H1):** a single `n_cmpl=N` request to llama.cpp-server reduces wall-clock time
for an N-candidate best-of-N round by ≥25% vs. N sequential `IInferencePort::complete()` calls to
Ollama, for both T2 (N=3) and T2.5 (N=5), with zero change in which candidates pass their fixture's
oracle (i.e., batching must not silently alter model outputs).

## Scope

- **Pilot tier:** T2 (`gemma4-12b`, N=3) — reuses `docs/benchmarks/fixtures/t2-*.json`
  (`t2-humanize-duration`, `t2-roman-to-int`), both `status: verified`, both already have working
  RED→GREEN oracles. No new fixture authoring needed.
- **Second tier, same run:** T2.5 (`devstral-small-2:24b` if VRAM allows — see Phase 0 — else
  `qwen2.5-coder:14b` as the stand-in already validated in the DSpark doc's Follow-up 3), N=5, using
  `docs/benchmarks/fixtures/t25-*.json`.
- **Out of scope this round:** wiring `complete_batch` into production `ScaffoldedDispatch` — that's
  the ADR-gated step if this passes. Also out of scope: cross-restart disk persistence of the slot
  cache (reame's failed feature) — the in-memory LCP slot cache is a separate, already-working
  mechanism this plan can observe as a side effect but isn't the primary measurement.

## Phase 0 — feasibility gate (measure, don't assume)

Known from source inspection, not yet verified at runtime:

- No `llama-server` binary exists anywhere on this box today (checked: `find` for `llama-server*`
  turned up nothing but a Python simulator script). Two paths, unevaluated:
  (a) build from the vendored source (`scripts/lora/llama.cpp`) via its existing CMake config, or
  (b) pull a prebuilt release binary, as the DSpark doc did for its one-off Vulkan test
  (`llama-b9857-bin-ubuntu-vulkan-x64`). **Do (b) first** — it's what already worked once on this box
  and avoids a first-time C++ build as a dependency of this measurement.
- No CUDA toolkit on this box (driver only, no `nvcc`; llama.cpp ships no prebuilt Linux CUDA binaries).
  The Vulkan backend is the only realistic option, per the DSpark doc's own finding — the NVIDIA Vulkan
  ICD (`libnvidia-gl-580`) is already present and gave real GPU acceleration there.
- Model files: point `--model` directly at Ollama's own blob store
  (`/usr/share/ollama/.ollama/models/blobs/sha256-*`) — confirmed world-readable, valid GGUF headers,
  and already used this way in the DSpark doc's Follow-up 2/3. No re-download needed.
- VRAM: re-check `devstral-small-2:24b`'s headroom (Phase 0 of the DSpark doc found only ~800MB free
  above its 15GB footprint) — N=5 slots each need their own KV-cache allocation on top of the shared
  weights, which may not fit even though the weights themselves did. If it doesn't fit, fall back to
  the `qwen2.5-coder:14b` stand-in immediately rather than fighting VRAM — don't burn Phase 1 time on a
  config that Phase 0 already rules out, per the DSpark doc's own precedent.
- Confirm `n_cmpl` is reachable from the JSON-RPC-style `/completion` endpoint on the actual built
  binary (source inspection can miss version drift between the vendored checkout and the release
  binary's cut) — a two-request smoke test (`n_cmpl: 1` then `n_cmpl: 3`, diff the response shape)
  before running real fixtures.

## Phase 1 — batched vs. sequential (isolate the batching effect from the backend-swap effect)

Three configs, same model, same fixtures — same isolation structure as the DSpark doc's Phase 1:

| Config | What it isolates |
|---|---|
| (a) Ollama, N sequential `complete()` calls (current production behavior) | baseline |
| (b) llama.cpp-server, N sequential single-completion requests (`n_cmpl=1` each) | backend-swap effect alone (expect ~16-19% slower per the DSpark doc's single-stream finding) |
| (c) llama.cpp-server, one `n_cmpl=N` request | batching effect on top of the backend-swap tax |

**Metrics per config, per fixture, n=5 repeats:**
- wall-clock for the full N-candidate round (not per-token tok/s — the unit that matters here is
  "time to get all N candidates back," matching what `ScaffoldedDispatch` actually waits on)
- peak VRAM resident (N=5 slots is a real allocation, not free)
- which candidates passed the fixture's oracle, compared 1:1 against config (a)'s pass/fail set on the
  same random seed — flag any divergence immediately, don't wait for Phase 2

## Phase 2 — correctness / regression gate

Run each fixture's already-defined oracle (`cargo_test`/`cargo_check` command in the fixture JSON)
against every candidate config (c) actually produced. Expect the same pass/fail pattern as config (a)
modulo normal sampling variance (temperature > 0 means candidate-level content will differ run to run
regardless of batching — the gate is "does `ScaffoldedDispatch`'s decision — did *any* candidate pass —
change", not "are the token strings identical"). Any systematic divergence (e.g. batched candidates
consistently worse) is a stop-ship signal independent of Phase 1's latency numbers.

## Phase 3 — concurrent load (conditional on Phase 1 passing)

Only run if Phase 1 shows a real per-round win. Fire 2 and 4 concurrent `ScaffoldedDispatch`-shaped
requests (proxy for real HexFlo swarm parallelism dispatching multiple tasks at once) against config
(c), and check whether `n_parallel` slot contention (5 slots per T2.5 request × concurrent requests)
degrades the win. This is where `project_nexus_arena_spin` (glibc arena contention under concurrent
load) is most likely to resurface — watch for it specifically, same caution as the DSpark doc's Phase 3.

## Decision gate

- **PASS** (≥25% real per-round wall-clock win, zero correctness regression, fits VRAM) → draft the ADR
  for a new `LlamaCppServerAdapter` (`hex-nexus/src/adapters/inference/llamacpp.rs`) implementing
  `IInferencePort`, plus the new `complete_batch(request, n)` trait method (default impl = today's
  sequential loop, so `OllamaInferenceAdapter`/`ClaudeCodeInferenceAdapter` need no changes) and its
  wiring into `ScaffoldedDispatch::dispatch()`. The ADR must also settle: does `hex nexus start`
  spawn/supervise `llama-server`, or is it an external prerequisite like Ollama; and does hex build the
  binary from vendored source or fetch prebuilt releases per-platform.
- **MARGINAL** (10-25%, or VRAM-tight requiring the 14b stand-in instead of the real T2.5 model) →
  record findings in project memory, shelve. Re-visit if VRAM headroom changes or a CUDA build path
  opens up (would likely close much of the backend-swap tax, per the DSpark doc's Follow-up 2 math).
- **FAIL** (<10%, any correctness regression, or a Phase 0 blocker with no fallback) → shelve, no ADR —
  same disposition as the vLLM path.

## Execution mechanics

- A single-purpose `scripts/benchmark-llamacpp-batched.sh` — dev-utility, allowed under the no-runtime-
  scripts rule's explicit exception (`scripts/benchmark-*.sh`), same category as the DSpark doc's
  `scripts/benchmark-vllm-specdecode.sh`. Throwaway measurement tool, not a permanent addition.
- If the decision gate passes, the durable version is the `--backend ollama|llamacpp` dimension on
  `hex config inference bench --agentic` (already flagged as follow-up work by ADR-2606071734), not the
  script kept around long-term.
- Local and read-only against this box's own GPU; no shared state, no destructive actions. The only
  real-world side effect is fetching a prebuilt `llama-server` release binary.

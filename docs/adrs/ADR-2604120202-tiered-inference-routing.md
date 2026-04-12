# ADR-2604120202: Tiered Inference Routing with Local Model Scaffolding

**Status:** Accepted
**Date:** 2026-04-12
**Drivers:** Research synthesis (2026-04-11) from three parallel studies on prompt compression, context optimization, and small-model code generation quality. Findings: local models at 32B reach ~85% HumanEval but collapse to ~20% on SWE-Bench repository-level tasks. The gap is closable for T1-T2 tasks via tier-based routing, compile-gate Best-of-N, GBNF grammar constraints, and KV-cache prefix sharing — all production-ready techniques that build on the standalone dispatch path (ADR-2604112000).
**Supersedes:** None (complements ADR-2604112000 standalone dispatch, ADR-2604101500 local-inference-first, ADR-040 inference routing)

<!-- ID format: YYMMDDHHMM — 2604120202 = 2026-04-12 02:02 local -->

## Context

hex's inference router (`hex-nexus/src/adapters/inference_router.rs`, ADR-040) can already select the best server by model availability, load, and agent affinity. ADR-2604112000 shipped `OllamaInferenceAdapter` and wired `route_request()` to dispatch through it. 12 models are registered on the operator's Bazzite box (Strix Halo, Vulkan GPU, 32 tok/s on 32B Q4).

But routing today is **model-blind**: every task gets the same model regardless of complexity. A typo fix consumes the same 32B model capacity as a multi-file feature. And one-shot dispatch to local models fails on anything beyond single-function generation because:

1. **No tier awareness** — the router doesn't know whether a task is T1 (rename) or T3 (implement a Rust trait with mocked HTTP tests). It always picks the "best available" model.
2. **No error recovery** — one-shot generation either works or doesn't. No compile-gate, no retry, no Best-of-N.
3. **Prompt bloat** — 1500-token agent prompts inline file contents that could be retrieved on demand, and share ~800 tokens of system context that could be KV-cached.
4. **No output constraints** — small models hallucinate output formats (extra prose around code, wrong commit message shape) that hex's pipeline rejects.

Research shows these four gaps account for the entire quality difference between local and frontier on T1-T2 tasks. Multi-file T3 tasks remain frontier-only for now.

### Benchmark basis for the tier boundaries

| Task complexity | 32B local (one-shot) | 32B local (scaffolded) | Frontier |
|---|---|---|---|
| T1: single-line edit, rename | ~95% | ~98% (trivial improvement) | ~99% |
| T2: single function/test | ~85% | ~93-95% (Best-of-3 + compile gate) | ~95% |
| T2.5: multi-function, 2-file | ~50% | ~70% (agent loop, devstral) | ~90% |
| T3: multi-file feature | ~20% | ~30% (not sufficient) | ~55%+ |

The scaffolded T2 pass rate (93-95%) is within striking distance of frontier T2 (95%). This is the key insight: **for T2 tasks, scaffolding closes the gap completely**. T3 remains frontier-only, and that's acceptable — T3 tasks represent ~30% of hex workplan steps.

## Decision

hex SHALL implement **tiered inference routing with local model scaffolding** in two phases:

### Phase 1: Tier-aware routing + prompt optimization (low effort, high impact)

1. **Task-tier field on CodeGenRequest.** Add `tier: Option<TaskTier>` (enum: T1, T2, T3) to `CodeGenRequest` in `remote/transport.rs`. The workplan executor populates this from the workplan task's `layer` + `description` complexity, or from the hook route T1/T2/T3 classifier that already exists (`hex-cli/src/commands/hook.rs::classify_work_intent`).

2. **Tier→model mapping in the router.** `InferenceRouterAdapter::route_request` reads the tier and overrides model selection:
   - T1 → `qwen3:4b` (fastest, ~100 tok/s, sufficient for trivial edits)
   - T2 → `qwen2.5-coder:32b` (best local codegen)
   - T2.5 → `devstral-small-2:24b` (best local agentic model)
   - T3 → frontier (ClaudeCodeInferenceAdapter) or return `Err` if no frontier available
   - Mapping is configurable via `.hex/project.json` → `inference.tier_models` so operators can override per-project.

3. **KV-cache prefix sharing.** Restructure agent prompt templates (`hex-cli/assets/agents/hex/hex/*.yml`) so the first ~800 tokens are identical across all agents of the same role. Ollama's `--keep-alive` caches this prefix automatically. No code change — prompt template restructuring only.

4. **Quantization-aware prompt restructuring.** Modify agent YAML `workflow.context` sections: flatten nested instructions to numbered steps, front-load file contents before instructions, repeat key constraints at prompt end. Based on empirical finding that Q4 models lose more on mid-sequence information and nested conditionals.

### Phase 2: Scaffolding layer (medium effort, high impact)

5. **Best-of-N with compile gate.** New orchestration component `ScaffoldedDispatch` in `hex-nexus/src/orchestration/scaffolding.rs`:
   - For T2 tasks: generate N=3 completions from the selected model.
   - Run `cargo check` (or language-appropriate type checker) on each completion.
   - Return the first that compiles. If none compile, escalate to frontier.
   - N is configurable per tier (T1: N=1, T2: N=3, T2.5: N=5, T3: N=1 frontier).
   - Integrated with the existing `validate_dispatch_evidence` guard from P6.

6. **Error-feedback retry loop.** On compilation failure (all N attempts fail), feed the best error message back to the model for up to 2 retries. Research shows 15-25% improvement at 32B scale; third retry almost never helps. Cap at 2 retries to bound latency.

7. **GBNF grammar constraints.** Add a `grammar: Option<String>` field to `OllamaInferenceAdapter::complete()` that passes through to Ollama's `/api/generate` `options.grammar` parameter. Define hex output grammars:
   - `code-only.gbnf`: emits a code block with no surrounding prose
   - `code-and-commit.gbnf`: emits `{"files": [...], "code": "...", "commit_msg": "..."}`
   - `analysis.gbnf`: emits structured markdown with required headings
   Grammar selection is driven by the task's `agent` field in the workplan (hex-coder → code-only, planner → analysis).

8. **Cascading escalation.** When a T2 task fails all N attempts + 2 retries, the scaffolding layer automatically escalates to frontier via `ClaudeCodeInferenceAdapter`. Track escalation rates per task-type in HexFlo memory. If a task-type escalates >50% of the time, the tier classifier should reclassify it as T3.

### Out of scope (deferred to future ADRs)

- **RAG for file contents** — requires a vector store and embedding pipeline. High value but separate infrastructure concern.
- **LLMLingua-2 prompt compression** — marginal after RAG is in place. Defer until RAG ships.
- **LoRA distillation from Opus traces** — highest ceiling but requires a training pipeline. Separate ADR when trace collection is mature.
- **Multi-turn decomposition** — breaking prompts into sub-steps is valuable but changes the agent execution model. Separate ADR for agent-loop architecture.
- **Speculative decoding** — 2-3x speedup but requires llama.cpp configuration, not Ollama API. Defer.

## Consequences

**What this ADR gives us:**
- ~70% of hex tasks route to local models instead of 0% today.
- T2 pass rate rises from ~85% (one-shot) to ~93-95% (scaffolded), matching frontier on single-function tasks.
- T1 tasks complete in <5 seconds (qwen3:4b at 100 tok/s) instead of API-round-trip latency.
- Frontier API calls drop to ~30% of tasks (T3 only + T2 escalation fallback).
- The tier→model mapping is data-driven: escalation tracking automatically refines boundaries over time.
- Prompt restructuring benefits ALL models (including frontier) by reducing token consumption.

**What this ADR costs us:**
- New orchestration surface: `ScaffoldedDispatch` (~300 lines), GBNF grammar files, tier field on CodeGenRequest.
- Compile-gate adds latency: T2 tasks take 30-90s (3 generations + 3 compile checks) instead of one-shot.
- GBNF grammars are model-specific: a grammar that works for qwen2.5-coder may need adjustment for devstral. Testing matrix grows.
- Escalation tracking requires HexFlo memory writes per dispatch — modest but nonzero overhead.

**What this ADR does NOT do:**
- Does not make T3 tasks work on local models. Multi-file features with architectural reasoning remain frontier-only. Research shows even 32B scaffolded only reaches ~30% on SWE-Bench — not sufficient for production.
- Does not remove the need for frontier model access. hex in standalone mode still needs either Claude API or Claude CLI for T3 tasks.
- Does not implement RAG, LoRA distillation, or multi-turn decomposition. Those are higher-infrastructure investments for future ADRs.

## Alternatives Considered

- **(A) Invest only in LoRA distillation (skip scaffolding).** Rejected: LoRA requires 200+ training examples and a QLoRA pipeline. Scaffolding is production-ready today and compound-effective immediately. LoRA can stack on top later.

- **(B) Use only prompt compression (LLMLingua-2) without tier routing.** Rejected: compression alone cannot fix the quality gap — a 32B model with a perfectly compressed prompt still fails at multi-file reasoning. Tier routing addresses the root cause (wrong model for the task complexity).

- **(C) Route everything through frontier and use local models only for drafts (speculative decoding).** Rejected: defeats the standalone mode purpose (ADR-2604112000). hex must work without any frontier access for T1-T2 tasks. Speculative decoding is a speedup optimization, not a routing strategy.

- **(D) Wait for better local models (70B+ quantized, or next-gen 32B).** Rejected: the scaffolding techniques improve ANY model at ANY scale. When better models ship, the same Best-of-N + compile-gate + retry loop makes them even better. The investment compounds rather than becoming obsolete.

- **(E) Build a full agent-loop framework (like OpenHands) for all tasks.** Rejected as overscoped: agent loops help T2.5 tasks but add 3-5x latency and token overhead. Best-of-N + compile gate achieves 80% of the benefit at 20% of the complexity. Agent loops can be added for T2.5 in a follow-up.

## Notes

- The tier→model mapping (`qwen3:4b` for T1, `qwen2.5-coder:32b` for T2, `devstral-small-2:24b` for T2.5) is based on current benchmarks and the operator's hardware. It should be treated as a default, not a constant — the `.hex/project.json` override and escalation tracking exist to let operators tune it.
- GBNF grammar support in Ollama is via the `options.grammar` field on `/api/generate`. This is a string containing the BNF grammar definition. llama.cpp's GBNF format is well-documented and Ollama passes it through directly.
- The Best-of-N compile gate reuses hex's existing `cargo check` infrastructure (via `hex validate` or direct subprocess). No new build tooling is needed.
- The workplan `wp-tiered-inference-routing` tracks execution. It is NOT P0-BLOCKER — hex works without it (via ADR-2604112000 standalone dispatch). This ADR is a quality-of-service improvement, not a correctness fix.
- Related: ADR-2604112000 (standalone dispatch — the infrastructure this builds on), ADR-2604101500 (local-inference-first — the policy this implements), ADR-040 (inference routing — the router this extends), ADR-2604102200 (RL self-improvement — the learning loop that will consume escalation data).
- Research report at `docs/analysis/research-local-model-optimization.md`.

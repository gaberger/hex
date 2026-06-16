# ADR-2606161300: Encoding hex into a model via LoRA — idiom injection, not constraint enforcement

**Status:** Proposed
**Date:** 2026-06-16
**Epoch:** single-agent (hybrid-inference)
**Drivers:** Operator question ("what if we encode hex architecture principles into the model itself"), prompted by *Decoupled Mixture-of-Experts for Parametric Knowledge Injection* (Yue, Su, Ai et al., Tsinghua, arXiv:2606.14243v1, Jun 2026). hex already runs local tier models (T1/T2/T2.5) and an RL improvement loop; the question is whether hex's own conventions can be baked into those models via LoRA rather than enforced purely externally.
**Related:** ADR-2026-05-22-1710 (codegen-tier-local-ollama), ADR-2026-04-12-0202 + ADR-2026-04-13-1630 (tiered inference routing), ADR-2606071734 (agentic-inference-benchmark-suite), ADR-2606072044 (evidence-gated best-of-N), ADR-2605301224 (dynamic loosely-coupled inference selection), ADR-2026-07-10-1000 (budget-aware dynamic routing)

## Context

### The question

hex enforces hexagonal architecture **externally**: hooks classify intent, `hex analyze` checks boundary rules, behavioral specs + property tests act as independent oracles, ADRs record decisions, and the SOP loop re-fires personas when escalation catches a gap. The generator (a Claude tier or a local Ollama tier) is treated as untrusted; correctness is adjudicated outside it.

The operator asks: can we instead **encode hex into the model's parameters** so the model natively emits hex-idiomatic, boundary-respecting code? We run local tier models we can fine-tune (qwen2.5-coder:32b, devstral-small-2:24b, qwen3:4b — ADR-2026-05-22-1710), so LoRA is mechanically available.

### What the DMoE paper actually offers

DMoE (arXiv:2606.14243) is a **knowledge-injection** architecture with four design moves directly relevant to us:

1. **Decouple experts AND router from a frozen base model.** Base `θ` is never modified; each knowledge unit becomes an independent LoRA expert `Δθᵢ` (rank 4, α=16). Effective params at decode: `θ_eff = θ + Σ Δθᵢ`. Experts add/remove/update in isolation — no catastrophic forgetting, no re-train of the base.
2. **Uncertainty-gated activation.** Experts fire only when token uncertainty `TUₜ = −Σ pₜ(v) log pₜ(v)` exceeds a threshold (τ=2.0). If the model is already confident, no expert loads.
3. **Training-free BM25 lexical router.** No learned router; an inverted index over each expert's text surrogate. Registering an expert = inserting a document.
4. **Final-layer-FFN-only attachment.** Experts attach only to the last transformer FFN, preserving KV-cache validity → 1.3–5.1× faster decode than dynamic-RAG (FLARE), 1.6–1.9× less GPU memory.

Results: best/tied on 11 of 14 metrics (Llama-3.2-1B, Qwen2.5-1.5B), honestly *not* dominant on every dataset.

### The critical distinction this ADR exists to record

DMoE's benchmarks are **factual recall** (HotpotQA, CWQ, Quasar-T, StrategyQA). An expert encodes *"this passage says X."* Its gating oracle — token uncertainty — spikes when the model **lacks a fact**.

hex's principles are **behavioral constraints**, not facts:
- "an adapter MUST NOT import another adapter"
- "all relative imports use `.js` extensions (NodeNext)"
- "`composition-root` is the ONLY file importing adapters"
- "NEVER `mock.module()` — use the Deps pattern (ADR-014)"

A model can be **serenely confident while emitting a boundary violation.** TU does not spike on a rule-break it is sure about. Therefore:

> **Knowledge injection (DMoE's domain) is the wrong mechanism for constraint enforcement (hex's domain).** Conflating the two would re-couple the generator with the judge and destroy hex's independent-oracle property — the very thing the "tests can mirror bugs" lesson warns against.

There is, separately, a striking **structural resonance**: DMoE *is* hexagonal architecture applied to model internals (frozen base = domain core; LoRA experts = swappable secondary adapters; BM25 router = composition root; uncertainty gate = tier escalation). We adopt the *shape*, but we are precise about what it can and cannot do.

## Decision

We will encode hex into local LoRA experts **for idiom injection only**, and we will **keep all hard correctness gates external and unchanged**. Concretely:

### 1. Scope: idiom-nudging, never enforcement

A hex LoRA expert exists to **raise the floor** — make the model's first draft more hex-idiomatic so fewer drafts get rejected. `hex analyze`, behavioral specs, property tests, and the evidence-gated best-of-N compile gate (ADR-2606072044) remain the **ceiling** and the sole arbiters of acceptance. No LoRA output is ever trusted because it was "trained on the rules."

**Invariant (HARD):** Removing every hex LoRA expert MUST NOT weaken any correctness gate. Experts are a generation prior, not a verifier.

### 2. Training corpus — built from artifacts hex already owns

Experts are trained on hex's own source-of-truth, partitioned into knowledge units (one expert per unit, DMoE-style decoupling):

| Expert | Knowledge unit (corpus) | Idiom it injects |
|---|---|---|
| `hex-boundaries` | ADRs + CLAUDE.md hexagonal rules + analyzer-passing exemplar diffs | layering, no cross-adapter imports, composition-root wiring |
| `hex-rust-idiom` | hex-cli/hex-nexus/hex-core idiomatic code, Deps-pattern tests | error handling, port traits, `.js` extension rule for scaffolded TS |
| `hex-testing` | Behavioral-spec + property-test exemplars | spec-first, Deps over `mock.module()` (ADR-014) |
| `hex-scaffold` | `hex-cli/assets/` templates, ports/adapters skeletons | project structure on `hex init` output |

Corpus construction follows DMoE's PRAG-style recipe: per unit, augment each artifact into instruction-style pairs (a paraphrase that preserves content + Q/A pairs). **No answer strings, no benchmark-specific data** — we are teaching style, not leaking test answers.

### 3. Mechanism — LoRA adapters on local tier models

- LoRA rank 4, α=16 as a starting point (DMoE's values; tune via the bench harness). Adapters attach to **final-layer FFN** to preserve KV-cache reuse during the agent ReAct loop (ADR-2606071500) — this is the paper's key efficiency result and matters because our loop decodes long, multi-turn sequences.
- Base tier models stay **frozen**. Adapters are versioned artifacts in the inference registry, swappable per-project (a target project can ship its own idiom expert without re-training a base).
- **Activation gate:** start with *always-on per-tier* (simplest). Adopt DMoE's uncertainty-gated triggering (TU>τ) as a Phase 2 optimization *only* if always-on measurably hurts throughput — and even then, the gate selects when to spend compute, never when to enforce a rule.

### 4. Routing reuses the existing tier router, not a new neural router

We do **not** build a learned router. The hex inference tier (driven by `strategy_hint`) already selects which model handles a task; the per-tier LoRA expert rides along. If multiple experts apply (e.g. Rust + testing), compose them (`θ + ΣΔθᵢ`) as DMoE does. This keeps routing fully decoupled and "training-free" in the paper's sense.

### 5. Evaluation is mandatory and external — the bench harness owns the verdict

No expert is promoted to a tier default until the **agentic inference benchmark suite** (ADR-2606071734) shows, against the un-adapted base on the same tier:
- **Acceptance-rate lift:** higher first-draft pass rate through `hex analyze` + compile gate (the real signal — fewer best-of-N rounds, ADR-2606072044).
- **No regression** on general codegen quality or cross-adapter reasoning.
- **No throughput regression** beyond an agreed budget (the bench already measures tok/s; reasoning-model truncation caveats per `project_nemotron3_ultra_bench` memory apply).

This guards against the known failure mode (memory `feedback_qwen36_not_for_codegen`): a model that *looks* better on a flawed harness. The harness, not the LoRA training loss, decides.

### 6. Explicit non-goals

- **NOT** parametric enforcement of boundary rules. That is a verification problem owned by `hex analyze` / RL reward signals / specs — DMoE provides zero evidence it transfers to constraint satisfaction.
- **NOT** modifying base-model weights (no full SFT — avoids catastrophic forgetting, per the paper's own RAG-vs-post-training framing).
- **NOT** a replacement for ADRs/specs as the durable record. The corpus is *derived from* those artifacts; they remain canonical.

## Consequences

### Positive
- Higher first-draft acceptance → fewer best-of-N rounds → lower local-inference cost and latency for T1/T2/T2.5 work.
- Experts are decoupled, versioned, per-project swappable; updating an idiom = retrain one small adapter + insert, never re-train a base (DMoE's modularity win).
- KV-cache-safe final-FFN attachment keeps the long ReAct loop efficient.
- Feeds the RL improvement loop (`project_rl_loop_stuck_fallback`): expert quality becomes another dimension the loop can evaluate across the local fleet.

### Negative / risks
- **Idiom drift:** a stale expert trained on superseded ADRs injects obsolete idioms. Mitigation: retrain triggers on ADR-epoch changes (ADR-2606071243); experts carry a corpus-version stamp.
- **False confidence:** the temptation to relax external gates because "the model knows the rules now." The HARD invariant (§1) and external-only verdict (§5) exist precisely to forbid this. This ADR is the written guard against that mistake.
- **Training/eval cost** on a 16GB-GPU/30GB-RAM box (`project_qwen_next_hardware_ceiling`): rank-4 LoRA on a 24–32B model is feasible but must respect the resource governor (ADR-2606080915). Large bases may need quantized or off-box training.
- **Marginal, not transformative:** DMoE itself reports non-dominant results. Expect a floor-raise, not a step-change. If the bench shows no acceptance lift, we abandon — this is a Proposed experiment, not a commitment.

### Neutral
- Reuses tier routing and the bench harness; no new router, no new verdict authority.

## Implementation

Phased; each phase gated by the bench harness. Decompose into a workplan (`hex plan draft`) before coding.

**Phase 0 — Corpus tooling.** A `hex` verb to extract a per-expert training corpus from ADRs/specs/exemplar diffs (instruction-style augmentation, PRAG recipe). Output is auditable artifacts, not opaque data. No runtime scripts (CLAUDE.md HARD RULE) — implement in hex-nexus / hex-cli.

**Phase 1 — Single expert, single tier.** Train `hex-boundaries` LoRA on the T2 model (qwen2.5-coder). Always-on. Wire adapter load into `inference-gateway` / hex-nexus HTTP path. Final-FFN attachment.

**Phase 2 — Bench gate.** Run the agentic benchmark suite (ADR-2606071734): base vs base+expert on first-draft acceptance rate, codegen quality, throughput. Promote only on lift + no regression. Record verdict in STDB (`hex memory store lesson:hex-lora-<expert>`).

**Phase 3 — Expert fleet + composition.** Add `hex-rust-idiom`, `hex-testing`, `hex-scaffold`. Compose multiple experts (`θ + ΣΔθᵢ`). Per-project adapter selection.

**Phase 4 (optional) — Uncertainty gating.** Only if always-on hurts throughput: add TU>τ triggering to spend adapter compute selectively. Never as enforcement.

**Phase 5 — RL loop integration.** Expose expert-vs-base acceptance lift to the RL improvement loop so it evaluates idiom experts across the full local fleet (extends `project_rl_loop_stuck_fallback` fix).

**Rollback:** every expert is removable with zero impact on correctness gates (the §1 invariant). Disabling = drop the adapter from the registry; base tier behavior is unchanged.

## References

- Yue, Su, Ai, Tang, Wang, Kang, Zhan, Liu. *Decoupled Mixture-of-Experts for Parametric Knowledge Injection.* arXiv:2606.14243v1, 12 Jun 2026.
- ADR-2026-05-22-1710 — codegen tier on local Ollama (the models we LoRA).
- ADR-2026-04-12-0202, ADR-2026-04-13-1630 — tiered inference routing (the router experts ride on).
- ADR-2606071734 — agentic inference benchmark suite (the external verdict authority).
- ADR-2606072044 — evidence-gated best-of-N compile gate (the ceiling experts must not weaken).
- ADR-2605301224 — dynamic loosely-coupled inference selection.
- ADR-2606071243 — ADR epochs (retrain-on-epoch-change trigger).
- ADR-2606080915 — resource governor (training admission control).
- ADR-014 — Deps pattern (idiom the `hex-testing` expert injects).
- CLAUDE.md — Key Lessons ("tests can mirror bugs"; independent oracles) — the principle this ADR's scope boundary protects.

# ADR-2606071734: Agentic inference benchmark suite — test the harness, not the prompt

**Status:** Completed
**Date:** 2026-06-07
**Epoch:** single-agent
**Drivers:** This session's live evidence: `qwen2.5-coder:14b` would score well on the existing
single-turn `hex config inference bench` (code-gen/reasoning/identity quality) yet **wandered 18
steps in the ReAct loop and shipped nothing**; the `--fast` single-shot path landed the identical
task on attempt 1. The current benchmark measures *can it write code*; it does **not** predict
*can it drive hex's agentic loop* — which is the capability that actually gates hex's usefulness.
**Supersedes:**
**Superseded-By:**

## Context

`hex config inference bench` (ADR-2026-04-13-1238, `neural_lab_quant.rs`) scores a model on
single-turn prompts → a quality number + tier recommendation. Necessary, but not predictive: the
controlled comparison we ran this session (same model/task/tools/context, only loop-vs-single-shot
changed) isolated the failure to **multi-turn tool-loop driving**, which single-turn benchmarks
cannot see. A model can be "great codegen, terrible agent" — and the only way we discovered that was
by watching it wander.

What hex actually dispatches local models to do is the **evidence-gated `hex do` loop**: reason →
read/verify tools → converge on `propose_edit` → pass an independent oracle → commit. The benchmark
must exercise *that*.

## Decision

Add an **agentic benchmark suite** that runs fixtures through the real `hex do` harness and scores a
**capability vector**, not a single number. It extends — does not replace — the single-turn bench.

**Six axes** (each measurable from one harness run):
1. **Tool-protocol fidelity** — well-formed tool calls, valid args, guard-respecting.
2. **ReAct convergence** — steps-to-first-`propose_edit`, did-it-ever-edit, wander rate.
3. **Evidence-gated success** — independent-oracle pass %, attempts-to-green; ReAct vs `--fast`.
4. **Graph-context utilization** — pass-rate Δ with vs without graph context (the thesis test; ties ⑥).
5. **Tier-fit** — separate fixtures per tier (T1 scaffold/transform, T2 codegen, T2.5 cross-cutting).
6. **Throughput economics** — tokens/s, VRAM fit, latency *per loop step*.

**Construction (route through hex, un-gameable):**
- Each fixture is a self-contained `hex do`-shaped case: `{instruction, target_file, oracle
  (setup_files + command the model never sees/edits), tier, axis_focus, graph_context, arms}`.
  Same un-gameable design as the proven `humanize_duration` run — the oracle is independent of the
  agent.
- The runner materializes a sandbox repo, lays down the oracle, runs the case through the real loop
  in each arm (`react`, `fast`) × graph `{on, off}`, and records `did_edit, steps_to_edit,
  evidence_pass, attempts, wall_ms, tokens, failure_reason`.
- `failure_reason` is exactly the telemetry ADR-2606071651 (①) adds; the graph arm is ⑥'s ablation —
  building the suite pulls both forward.
- Every run is baselined against a **frontier ceiling** (Claude via the ⑤ wire, once it exists). The
  gap-to-frontier — not the raw score — is what tells you if a model is good enough for a tier.

**Surface:** `hex config inference bench --agentic [--corpus docs/benchmarks] [--arm react|fast]
[--graph on|off] [--compare <baseline>]`. NOT a standalone script (no-runtime-scripts rule). Results
persist to STDB like `q-report`/`usage`.

**Corpus location:** `docs/benchmarks/` (versioned, in-repo). NOT `hex-cli/assets/` — the fixtures
reference hex-intf internals (`hex-cli/src/fmt.rs`, etc.), which the embedded-assets generic-only rule
forbids.

## Consequences

**Positive**
- Predicts hex usefulness, not Leetcode skill. Would have flagged qwen2.5-coder:14b as
  "T2-codegen OK / agentic FAIL" — the truth we only found by watching.
- Reports a vector, so "good codegen, bad loop" is visible instead of hidden behind one score.
- Makes model-selection per tier an evidence decision; makes the harness-vs-model wander question
  (this session's open debate) empirically settleable via the 2×2 {model}×{nudge}.

**Negative / risks**
- A real harness run per fixture is slower/costlier than a single prompt — keep the corpus small and
  curated (~12), not exhaustive.
- Fixtures rot as the codebase moves (a fixture targeting `fmt.rs` breaks if `fmt.rs` is refactored) —
  versioned corpus + a `status: verified|draft` field + CI that re-verifies oracles.
- Full value needs ⑤ (frontier baseline) and ① (failure_reason); the corpus + schema land now and the
  runner fills in as those arrive.

## Implementation

- `docs/benchmarks/README.md` — corpus schema + scoring-vector definition + run protocol. *(this ADR)*
- `docs/benchmarks/fixtures/*.json` — seed corpus; fixture #1 = `humanize_duration` (verified, with
  observed baseline). *(this ADR)*
- `hex config inference bench --agentic` — the runner: sandbox materialize → real-loop dispatch per
  arm → vector score → STDB persist. *(follow-up workplan)*
- CI gate: re-verify every `status: verified` oracle still RED→GREENs on a frontier model, so the
  corpus can't silently rot.

Tracking workplan: create via `hex plan draft`.

## References

- ADR-2026-04-13-1238 — the single-turn `bench` this extends.
- ADR-2606071651 — `failure_reason` telemetry (the runner consumes it).
- ADR-2606071713 — graph-as-harness-tool (the graph-on/off ablation arm).
- Memory: `project_inference_tiers` (current tier scores this suite will supersede with agentic data).
- Live evidence: ReAct `loop ended with no edit · 18 steps` vs `--fast` `pass · 1 attempt · 49fc3b09`.

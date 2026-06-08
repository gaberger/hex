# ADR-2606072044: Evidence-gated best-of-N across complementary models in the do-loop

**Status:** Accepted
**Date:** 2026-06-07
**Epoch:** single-agent
**Drivers:** A measured 3×3 benchmark grid (3 verified fixtures × 3 local models, react n=5)
showed **no single local model dominates, and the models have complementary blind spots**:
devstral aces strings/loop-driving but fails stack-algorithms (rpn 1/5); qwen aces algorithms
but fails string-state-machines (csv 0/5). Picking a single `react_model` reorders the "best"
model on every new fixture (the default flipped three times). Meanwhile the do-loop uses ONE
fixed model and hex's existing tier routing is compute-budget-based, single-model-per-tier, and
wired to the retired workplan/SOP path — not the single-agent loop.
**Supersedes:**
**Superseded-By:**

## Context

The 3×3 grid (`docs/benchmarks/`, `hex bench agentic`, react n=5):

| Model | humanize (trivial) | rpn (stack algo) | csv (string SM) | total |
|---|---|---|---|---|
| devstral-small-2:24b | 5/5 | **1/5** | 5/5 | 11/15 |
| qwen2.5-coder:14b | 3/5 | 4/5 | **0/5** | 7/15 |
| gemma3:12b | 0/5 | 2/5 | 4/5 | 6/15 |

The two strong models have **anti-correlated failures**: devstral's hole is qwen's strength
(rpn) and qwen's hole is devstral's strength (csv). A single default *always* inherits one
model's catastrophic blind spot — we measured two different ones. No amount of default-tuning
fixes this; the ranking is unstable because the question "which model is best" has no
task-independent answer.

What hex has today does not address it:
- The **single-agent do-loop** (`hex-exec`, the canonical `hex do` path) has **no routing** —
  `resolve_react_model` is a flat config lookup. One model per run, regardless of task.
- The **tier router** (`task_type_classifier`, `quant_router`) classifies by *compute budget*
  (5 coarse types → tier → one `tier_models` entry) and is called only by `sop_executor` /
  `workplan_executor` — the org-sim path, not the do-loop. It cannot express "this is a
  string task → devstral" vs "this is an algorithm → qwen."
- **Best-of-N** exists (scaffolded dispatch) but samples the *same* model N times + escalates
  to *one* frontier — never selects across different local models.

Crucially, the single-agent epoch already has the perfect arbiter for "which model's output is
right": **the evidence gate.** We don't need a clairvoyant classifier — we can run candidates
and let the gate, which already decides what commits, pick the winner.

## Decision

Add **evidence-gated best-of-N across a small set of complementary models** to the do-loop, with
optional **skill-routing** as a cheap ordering heuristic (not a hard gate).

1. **Candidate set, not a single model.** `resolve_react_model` becomes `resolve_react_models`
   returning an ordered list (default `[devstral-small-2:24b, qwen2.5-coder:14b]` — the measured
   complementary pair; configurable via `.hex/project.json` `inference.react_models`). A single
   `react_model` stays valid (list of one) for cost-sensitive runs.

2. **The evidence gate is the selector.** Run the task on candidates and **commit the first whose
   `propose_edit` passes the evidence command**. This needs no classifier to be correct — the gate
   already is the source of truth. Because the candidates' blind spots are anti-correlated, the
   pair covers cases neither covers alone (devstral's rpn 1/5 + qwen's rpn 4/5 → the pair passes
   rpn; qwen's csv 0/5 + devstral's csv 5/5 → the pair passes csv).

3. **Skill-routing orders the candidates, doesn't pick.** Optionally extend the task-type signal
   with a *skill* axis (e.g. `string-processing`, `algorithmic`, `mechanical`) to decide which
   candidate to *try first* (cheaper on the happy path). A mis-route only costs latency — the gate
   still catches it and the next candidate runs. So routing is an optimization, never a correctness
   dependency.

4. **Record the winner.** Log which model passed per task into the run feed; over time this is the
   training signal for the skill-router (and for trimming the candidate set per project).

Execution order is configurable: **sequential** (try first candidate; on no-pass, try next — lower
cost, higher latency) or **parallel** (race both, take first pass — higher cost, lower latency).
Default sequential, skill-ordered.

## Consequences

**Positive**
- Eliminates the single-model blind spot the grid exposed — the pair's measured coverage is
  strictly better than either alone.
- No reliance on a perfect classifier: the **evidence gate already arbitrates**, so a wrong route
  degrades to latency, never to a bad commit.
- Composes with the loop-guard (ADR-2606071651): a candidate that wanders/`max_steps` simply yields
  to the next candidate — cross-model fallback generalizes the single-shot fallback.
- Generates the data (per-task winners) to make routing smarter without guessing up front.

**Negative / risks**
- Cost: up to N× inference on the failure path (sequential bounds this to "until first pass";
  parallel pays N× always). Mitigated by skill-ordering (happy path = 1×) and a small N (2).
- Latency on sequential misses. Mitigated by the parallel mode for interactive use.
- More moving parts in the loop; candidate-set config must stay simple (a list).
- Two models resident may exceed VRAM if both are large — the default pair (15.2 GB + 9 GB) won't
  co-reside on 16 GB, so sequential (load/evict) is the safe default; parallel needs headroom or
  one cloud candidate.

## Implementation

- `hex-exec`: `resolve_react_models(task) -> Vec<String>` (from `inference.react_models`, else the
  single `react_model`, else default pair); drive the loop per candidate; commit on first
  evidence-pass; return the winning model in the run feed.
- `.hex/project.json`: `inference.react_models: ["devstral-small-2:24b", "qwen2.5-coder:14b"]` and
  `inference.react_select: "sequential" | "parallel"`.
- Optional skill signal: extend `task_type_classifier` (or a lightweight do-loop-local heuristic)
  with a skill axis to order candidates; advisory only.
- Validate via `hex bench agentic`: the **pair** should score ≥ max(individual) on every fixture
  in the grid — the corpus is the regression test for the router.

Tracking workplan: create via `hex plan draft`.

## References

- ADR-2606071651 — ReAct loop-guard (single-shot fallback; this generalizes fallback across models).
- ADR-2606071734 — agentic benchmark suite (the grid that motivates and will validate this).
- ADR-2026-04-12-0202 / ADR-2026-04-13-1630 — tier routing (compute-budget; this is orthogonal,
  skill-based, and on the do-loop instead of the workplan path).
- Live data: `docs/benchmarks/fixtures/` — devstral 11/15, qwen 7/15, gemma3 6/15, anti-correlated
  failures (devstral rpn 1/5, qwen csv 0/5).

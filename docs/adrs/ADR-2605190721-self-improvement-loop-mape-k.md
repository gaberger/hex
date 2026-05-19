# ADR-2605190721: Self-Improvement Loop — Closing MAPE-K (Plan / Execute / Knowledge)

**Status:** Proposed
**Date:** 2026-05-19
**Drivers:** README.md §Status names "the chain that closes the operator-asks-nothing loop" as the active development frontier. The `M`onitor + `A`nalyze halves (detectors → hypotheses) are live; the `P`lan, `E`xecute, and `K`nowledge halves are scaffolded but not wired end-to-end. `hex sched scores` returns *"No scores yet. Sched service is learning."* — the K phase has no data. Without it, the improver can fire but cannot get better.

## Context

The README claims a working MAPE-K control loop. As of 2026-05-19 the actual state is:

| Phase | Mechanism | Status | Verifier |
|---|---|---|---|
| **M**onitor | `tick_improver` reads ADR registry + STDB telemetry every 30s | ✓ live | `hex sched daemon-status` → `running pid=*` |
| **A**nalyze | `discover.rs` emits `Hypothesis { id, source, scope, severity, evidence }` | ✓ live, **single-source** | `hex sched improver discover --once` → 72 hypotheses today, **all sourced from `AdrDoctor`**. The hex-analyzer detectors (`god_types`, `cohesion`, `composition_churn`, `duplication`) are not wired. |
| **P**lan | `adversarial_swarm.rs::propose_strategic` spawns N=3 strategic variants at T2.5 | scaffolded — `unimplemented!()` or stub | no command surface yet |
| **E**xecute | `improver_judge.rs` 5-axis rubric + `improver_act.rs` shadow-promotion | scaffolded; `judge` returns hardcoded score; `act` writes to `docs/workplans/rejected/` only | no end-to-end test |
| **K**nowledge | `improver_event` STDB rows; `historical_reject_rate` axis | table exists, never written | `hex sched scores` → empty |

This ADR was named by README.md §Status as `ADR-2026-04-27-1100` but never written. This ADR replaces that reference.

## Decision

Close the loop in **5 phases**, each independently testable. Implementation lives in **wp-sched-improver-propose-judge-act** (see References).

**P1 — `propose_strategic` (PLAN)**. For each hypothesis, spawn N=3 LLM variants at T2.5 with distinct strategy prompts:

- `conservative` — minimum-viable patch; preserves all public API; touches ≤ 5 files.
- `aggressive` — redesign; willing to change adapter boundary; touches ≤ 20 files.
- `targeted` — narrow fix for the literal finding; no broader refactor.

Variants are emitted as `proposed_action` STDB rows with `related_hypothesis_id`. Inference uses devstral-small-2:24b (T2.5 default per ADR-2026-04-12-0202).

**P2 — Structured Judge (EXECUTE/decide)**. A deterministic rubric scores each variant on 5 axes:

1. **alignment** — does the variant address the hypothesis's stated evidence?
2. **blast_radius** — how many files touched? (lower better)
3. **dependency_satisfaction** — are all `informed_by` ADRs in `Accepted` status?
4. **reversibility** — can a single `git revert` undo? (true/false)
5. **historical_reject_rate** — how often has the operator overruled variants from this source/scope?

Scoring is computed in Rust (no LLM), with weights stored in `.hex/improver-weights.toml` so the operator can tune.

**P3 — Act via Shadow-Promotion (EXECUTE/apply)**. Winning variant is applied to a worktree branch `sched/improver/<hyp_id>`. Tier-A auto-merges; Tier-B drafts to `docs/workplans/drafts/` + P2 inbox; Tier-C halts at P1 inbox. Losers append to `docs/workplans/rejected/<hyp_id>-<variant>.json` for audit + the `historical_reject_rate` lookup.

**P4 — K phase (KNOWLEDGE)**. Every transition writes an `improver_event` row:

```
improver_event { hyp_id, phase, variant_id?, score?, action?, verdict, ts }
```

`hex sched scores` queries this table to compute per-source-pattern reject rates. The judge's `historical_reject_rate` axis reads the same table — closing the learning loop.

**P5 — End-to-end smoke**. A synthetic hypothesis (e.g. "ADR with status=Draft") fires through all 5 phases; integration test asserts that `sched scores` is non-empty after the run and the chosen variant landed on a shadow branch.

## Consequences

- The operator-asks-nothing loop becomes a documented, replayable code path. Today the only autonomous artifacts are the AdrDoctor auto-fixes (Tier-A, ~5 per day). After this lands, every Hypothesis from any detector can flow end-to-end without operator intervention.
- The `historical_reject_rate` axis only becomes meaningful after ~50 improver_event rows exist. Expect P4–P5 verifications to need a few weeks of background firing before the K phase carries signal.
- Cost ceiling: N=3 T2.5 calls per hypothesis × 5 hypotheses per tick × every 30s = up to 1,800 T2.5 inferences per hour. At qwen2.5-coder:14b local rates (~$0/call) this is free; on OpenRouter fallback (~$0.001/call) ~$1.80/hr peak. Operator-tunable via `HEX_IMPROVER_HYPOTHESIS_BUDGET`.
- Failure modes:
  - LLM returns invalid JSON for variant → caught at proposed_action insert, variant skipped.
  - All 3 variants tie on rubric → tie-breaker is `targeted` (most conservative).
  - Judge weights misconfigured → operator override via `.hex/improver-weights.toml`.

## Implementation

See `docs/workplans/wp-sched-improver-propose-judge-act.json` for the phase decomposition. Each phase is independently shippable; the loop is closed after P3 lands.

### Operator-only wire-up (one line, after P1 ships)

The workplan can't include this step because `hex-nexus/src/orchestration/sched.rs` is on the SafeFileWriter protected-files list (per `hex-core/src/domain/validation.rs::CRITICAL_FILES`) — autonomous agents cannot edit it. After P1.1–P1.3 land, the operator manually adds the call site:

```rust
// hex-nexus/src/orchestration/sched.rs, inside tick_improver():
for hyp in hypotheses {
    let _ = crate::orchestration::dispatch_propose_for_hypothesis(hyp).await;
}
```

This is a deliberate guardrail, not a bug: the loop refusing to autonomously rewrite its own dispatcher is the whole point of the protected-files list. The single-line edit takes ~30 seconds and gives the operator a clear injection point to audit before the autonomy loop closes.

## References

- ADR-2026-04-12-0202 (Tiered Inference Routing) — T2.5 model selection for the propose phase.
- ADR-2026-04-13-2300 (Brain Daemon Loop) — the tick mechanism this ADR extends.
- ADR-2026-04-26-1311 (Six-Layer Governance) — shadow-promotion contract used by P3.
- ADR-2026-04-27-1200 (Architectural Health Detectors) — sibling work that produces additional hypothesis sources for P1.
- ADR-2605190720 (Evidence Gate) — preconditions for trusting variant `done` claims.
- Workplan: `docs/workplans/wp-sched-improver-propose-judge-act.json`.
- Code (current scaffolds):
  - `hex-cli/src/commands/sched/improver/discover.rs` (live)
  - `hex-nexus/src/orchestration/adversarial_swarm.rs` (stub)
  - `hex-nexus/src/orchestration/improver_judge.rs` (stub)
  - `hex-nexus/src/orchestration/improver_act.rs` (stub)

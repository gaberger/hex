# Scheduled Tier Rebench

**Owner:** CTO (with COO sign-off on cadence/cost)
**Triggered by:** [[lesson:tier-pin-rebench]] — gemma4:latest silently regressed from 0.92 → 0.17 between 2026-05-04 and 2026-05-13 while pinned as T2/T2.5. Stale pin actively harmed agent work for ~9 days.
**Date:** 2026-05-13

## Problem

`hex inference bench` exists and is reliable, but it's only invoked manually. Models on the Ollama registry get retagged silently; bench prompts get tightened in hex-cli releases. Either change can swing a pinned tier model from "perfect" to "unusable" without any signal to the operator. Today the only detection path is "agent work feels slower / answers got worse" — which is exactly the silent-tool-failure pattern the tool-czar persona is meant to catch.

## Goal

Run `hex inference bench --save` against every model referenced by `.hex/project.json → inference.tier_models` on a recurring cadence. Compare each new score against the previous saved score. When a tier-pinned model drops by ≥0.10 OR falls below 0.50 absolute, raise a priority-2 inbox notification and a board ask to the tool-czar persona. **Do not auto-flip pins** — the bench grader can't catch tool-call schema drift, so flips need a real SOP-run verification first.

## Non-Goals

- Auto-flipping `.hex/project.json` pins (operator decision; lesson explicit on this).
- Benching every registered provider — only models referenced by `tier_models` (T1, T2, T2.5, T3 if Ollama-backed).
- Pulling new models speculatively.
- Replacing the manual `hex inference bench` verb — keep that for one-off operator runs.

## Implementation Surface

Per [[feedback_supervisor_in_stdb]], the cadence lives in STDB, not a nexus tokio loop or shell script:

1. **STDB schema** (in `spacetime-modules/hexflo-coordination/`):
   - `inference_bench_tick_schedule` table (mirrors `supervisor_tick_schedule` shape) — interval `7 * 24 * 3600` seconds default, configurable.
   - `inference_bench_history` table — `(provider_id, model, score, tok_s, code_gen_quality, reasoning_quality, run_at, run_id)`. Append-only; read by the regression detector and by the dashboard.
   - `inference_bench_alert` table — `(model, prev_score, new_score, delta, severity, raised_at, ack_at, ack_by)`.
   - Reducer `inference_bench_tick` — fires on schedule, writes a `bench_request` row that nexus polls (nexus owns HTTP, WASM doesn't).
   - Reducer `record_inference_bench_result(provider_id, model, ...metrics)` — appends to history, computes delta vs last result, emits alert row if threshold breached.

2. **Nexus side** (`hex-nexus/src/orchestration/`):
   - New `InferenceBenchExecutor` watches `bench_request` rows, invokes the existing `hex inference bench --save` code path (NOT subprocess — call the underlying function directly), pushes results back via `record_inference_bench_result`.
   - On alert-row insert, raises a priority-2 inbox notification (existing `hex inbox notify` path) + sends a board ask to the tool-czar persona via the existing org-message route.

3. **CLI surface**:
   - `hex inference bench-history [--model <m>]` — print recent runs.
   - `hex inference bench-cadence get|set <interval>` — operator override.
   - `hex inference bench-alerts list|ack` — see/clear unacked regressions.

4. **Dashboard** (`hex-nexus/assets/`):
   - New panel under the inference page showing per-model score trend (sparkline) over last N runs.
   - Red badge on any tier model with an unacked alert.

## Trigger Conditions

The tick reducer also fires (in addition to the cadence) when:
- Ollama version changes (nexus reads `ollama --version` on startup; on change, inserts an immediate `bench_request` for every tier-pinned model).
- A model's Ollama digest changes (nexus periodically `ollama list`s; digest-mismatch since last bench → immediate `bench_request` for that model).

## Acceptance Criteria

1. After a fresh `hex nexus start`, `inference_bench_tick_schedule` is seeded with default 7-day interval.
2. Manually fast-forwarding the schedule (or calling the reducer directly) runs the bench, writes a row to `inference_bench_history`, and (if applicable) `inference_bench_alert`.
3. Synthetic regression test: rebench gemma4 (or a mock provider returning intentionally-bad output), confirm priority-2 inbox notification fires AND the tool-czar persona receives a board ask with the diff.
4. Pin is **NOT** auto-flipped — `.hex/project.json` unchanged after the alert.
5. `hex inference bench-history --model qwen2.5-coder:14b` returns at least the 2026-05-13 baseline + the synthetic-regression run.
6. Dashboard sparkline renders without errors and shows the trend.

## Out-of-Scope (Followup Specs)

- A `tool-czar` "auto-suggest replacement" workflow — when an alert fires, the persona could automatically bench the top 3 candidates of similar size and propose a swap (still operator-approved).
- Cross-host bench aggregation (we have one Strix Halo box today; once a second host exists, history rows need a `host` discriminator).

## Dependencies

- ADR needed for the new STDB tables + reducers (CTO).
- Existing: `hex inference bench --save` (works), inbox priority-2 path (works), org-message routing to personas (works), `hex memory store` for lessons (works).

## Estimated Cost

- ~3 hex-coder tasks (STDB schema + reducers, nexus executor, CLI verbs).
- ~1 hex-coder task for dashboard panel.
- ~1 integrator task to wire the alert → tool-czar route.
- Bench runtime per cycle: ~2 min/model × 4 tier models = ~8 min CPU/wk. Negligible.

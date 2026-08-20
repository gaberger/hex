# ADR-2026-07-10-1000 — Resilient Dynamic Inference Routing (Budget-Aware, Fallback, Escalation)

Status: **Accepted**
Date: 2026-06-05
Applies-To: hex-nexus/src/adapters/inference_router/, hex-nexus/src/quant_router.rs, hex-nexus/src/rate_limiter.rs, hex-nexus/src/orchestration/sop_executor.rs

## Context

Inference tiers (`t1`/`t2`/`t2.5`/`t3`) are configured as a **single hardcoded model per tier** in `.hex/project.json` → `inference.tier_models`. Two problems follow:

1. **Manual rewiring + budget exposure.** Changing a tier means hand-editing JSON and bouncing nexus; there is no spend ceiling, so cloud providers can drain budget unchecked. Cost is *tracked* (`CostTracker` in `rate_limiter.rs`, surfaced by `hex config inference stats`) but never *acted on*.

2. **No graceful degradation — proven by incident 2026-06-05.** While authoring this very ADR, the cloud reasoning provider (Tenstorrent/DeepSeek-R1) returned `503 no_server`, and the local fallback (gemma4-12b) was too weak to satisfy the `adr_draft` validator. The SOP reasoning path (`sop_executor.rs::tier_model_for_intent`) reads one model per tier and calls it **directly**, *bypassing* `quant_router`'s existing local-first / healthy-provider selection and circuit breaker. Result: the run retried a dead provider 15 times, then dead-ended (`emitted=None`) with no fallback and no escalation. The factory could not author its own fix because of the exact gap the fix closes.

This supersedes the scope of the proposed budget-only routing work and folds in resilience (provider fallback + capability escalation). Builds on ADR-030 (Multi-Provider Inference Broker), ADR-031 (RL-Driven Model Selection & Token Budget Management), ADR-2026-04-10-1500 (Local-Inference-First), ADR-2026-04-05-2125 (Free-Tier Routing), and ADR-2026-03-27-1000 (Quantization-Aware Routing).

## Decision

1. **Per-tier candidate pools.** `inference.tier_models` accepts either a scalar model string *or* an ordered array per tier, e.g. `"t2.5": ["gemma4-12b", "deepseek-ai/DeepSeek-R1-0528"]`. Candidates are tried in order, local-first, falling through on failure or over-budget. A scalar auto-wraps to a single-element list (backward compatible).

2. **Budget guard.** New `inference.budget` block: `{ daily_usd_cap, monthly_usd_cap, mode: warn|demote|block }`, wired to the existing `CostTracker`. As projected spend nears a cap, provider selection filters out paid providers and auto-demotes cloud→local, preferring free-tier models. Phased: `warn` (log only) → `demote` (silent fallback) → `block` (hard fail with remediation hint).

3. **SOP path routes through `quant_router`.** `sop_executor` tool calls dispatch via `quant_router::select_provider_with_rate_limits` instead of a direct single-model call, so every persona/tool invocation inherits healthy-provider fallback + the circuit breaker. No more dead-ending on a single 503'd or saturated provider.

4. **Escalate on repeated validation failure.** When a typed tool (`adr_draft`, `spec_draft`, `code_patch`, …) fails its Phase-4 validator N times (default 3) on a given model, the reason loop escalates to the next candidate in the tier pool (a stronger model) rather than retrying the same incapable model to exhaustion.

## Consequences

- **Backward compatible** — existing scalar `tier_models` configs are unchanged in behavior (auto-wrap).
- **Self-healing** — a provider outage or an under-powered model degrades gracefully instead of stalling the factory.
- **Budget-safe** — cloud spend is bounded by an explicit, configurable ceiling; high-volume codegen stays local/free while rare high-value reasoning can use cloud under the cap.
- **Observable** — routing logs which candidate actually served a request and records every demotion/escalation (no silent quality changes).
- **Tested** — coverage added to `hex-nexus/tests/tier_routing.rs` for: scalar↔array parsing, ordered fallback, over-cap demotion, and escalation-on-validation-failure.

## Alternatives Considered

- **Status quo (single model per tier).** Rejected — the source of both the manual-rewiring toil and the 2026-06-05 stall.
- **RL-only model selection (ADR-031) without explicit pools/caps.** Rejected as insufficient alone — RL optimizes choice over time but gives operators no declarative control or hard budget ceiling, and does not address the SOP-bypasses-router gap.
- **Hard-block-only budget enforcement.** Rejected as too brittle — blocking mid-task strands work; `demote` preserves progress by falling back to local, with `block` available as an opt-in for strict environments.

## References

ADR-030, ADR-031, ADR-2026-04-10-1500, ADR-2026-04-05-2125, ADR-2026-03-27-1000.
Lessons: `lesson:sop-no-fallback-routing`, `lesson:persona-cooldown-no-override`, `lesson:phase4-verifier-passes-malformed-adr`.

# ADR-2026-07-10-1000 — Budget-Aware Dynamic Inference Routing

Status: **Proposed**
Date: 2026-06-05

## Context
Manual tier_model pinning causes cloud budget overruns and stale local-first fallbacks. Existing A tracking (ADR-031) lacks proactive semantics.

## Decision
1. ** perative per-tier candidate pools to .hex/project.json: inferenceinference.tier_models` accepts scalar OR ordered array per tier (e.g., `t2.5: [gemma4-12b, deepseek-ai/DeepSeek-R1-0528]`).
2. Implement `inference.budget` config with `daily_usd_cap`, `monthly_usd_cap`, and `mode: warn|demote|block`. Wire CostTracker to filter providers when near cap caps, preferring free-tier models.
3## Consequences
- Backward compatible ( (scalar auto-wraps)
- Phased enforcement (warn → demote → block)
- Silent demotion logs model fallbacks
- Test coverage in tier_routing.rs
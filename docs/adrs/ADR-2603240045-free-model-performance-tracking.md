# ADR-2603240045: Free Model Performance Tracking in SpacetimeDB

**Status:** Proposed
**Date:** 2026-03-24
**Drivers:** hex dev now auto-falls back to free OpenRouter models (`openrouter/free`) when paid credits run out. Free models vary wildly in quality — some produce compilable hex-compliant code, others don't. We need to track per-model success rates in SpacetimeDB so the RL engine can learn which free models work and which don't.

## Context

With the auto-router (`openrouter/auto` and `openrouter/free`), OpenRouter picks the actual model. The response includes which model was used (e.g., `qwen/qwen3-coder:free` or `nvidia/nemotron-3-super-120b-a12b:free`). This is critical data:

- Did the code compile?
- Did tests pass?
- What was the hex analyze score?
- How long did it take?
- What was the actual model used (not just "openrouter/free")?

Today this data exists in local session files (`tool_calls[]`) but not in SpacetimeDB. The RL engine (ADR-031) can't learn from it.

### What "learning" means here

The RL engine should discover patterns like:
- `qwen/qwen3-coder:free` produces compilable TypeScript 85% of the time
- `nvidia/nemotron-3-super-120b-a12b:free` is better for ADRs (reasoning) than code
- Free models need 2.1 fix iterations on average vs 0.8 for paid models
- Certain free models fail on hex boundary rules consistently

This data feeds back into model selection: even with `openrouter/auto`, hex can add `model` preferences in the request body to steer toward known-good models.

## Decision

### 1. Track Actual Model in Tool Calls

The `ToolCall` struct and `dev_tool_call` SpacetimeDB table already have a `model` field. Ensure it stores the **actual model used** (from the OpenRouter response `model` field), not the requested model (`openrouter/auto`).

```rust
// In the inference response parsing:
let actual_model = resp["model"].as_str();  // e.g. "qwen/qwen3-coder:free"
// NOT the requested model "openrouter/auto"
```

### 2. New SpacetimeDB Table: `model_performance`

```rust
#[spacetimedb::table(name = model_performance, public)]
pub struct ModelPerformance {
    #[primary_key]
    #[auto_inc]
    pub id: u64,
    pub model_id: String,          // "qwen/qwen3-coder:free"
    pub task_type: String,         // "reasoning", "code_generation", etc.
    pub phase: String,             // "adr", "workplan", "code", "fix"
    pub language: String,          // "typescript", "rust"
    pub success: u8,               // 1 = compiled/passed, 0 = failed
    pub compile_pass: u8,          // 1/0
    pub test_pass: u8,             // 1/0
    pub analyze_score: u32,        // hex analyze score (0-100)
    pub violations: u32,           // boundary violations count
    pub tokens: u64,
    pub cost_usd: String,          // "0.0000" for free models
    pub duration_ms: u64,
    pub fix_iterations: u32,       // how many fix attempts needed
    pub is_free: u8,               // 1 if free model, 0 if paid
    pub project_id: String,        // which project
    pub created_at: String,
}
```

### 3. Record Performance After Quality Gate

After the quality loop completes, record one `model_performance` entry per inference call with the quality gate results:

```
Code step used qwen/qwen3-coder:free
  → compile: PASS
  → test: N/A
  → analyze: 92/100, 0 violations
  → fix iterations: 0
  → SUCCESS

Code step used nvidia/nemotron-3-nano-30b:free
  → compile: FAIL (3 errors)
  → fix iteration 1: deepseek-r1 fixed 2 errors
  → fix iteration 2: deepseek-r1 fixed 1 error
  → compile: PASS
  → analyze: 78/100, 2 violations
  → fix iterations: 2
  → PARTIAL SUCCESS
```

### 4. RL Reward Signal Enhancement

Update the RL engine's reward signal (ADR-031) to incorporate free model performance:

```
reward = base_reward(success)
       + quality_bonus(analyze_score / 100)
       - fix_penalty(fix_iterations * 0.1)
       + cost_efficiency(1.0 if is_free else (1.0 - cost_usd * 10))
       + speed_bonus(1.0 - duration_ms / 120000)
```

Free models that produce Grade A code on first try get maximum reward — the RL engine learns to prefer them over paid models that aren't significantly better.

### 5. Model Preference Injection

OpenRouter's auto-router accepts `model` preferences in the request body:

```json
{
  "model": "openrouter/auto",
  "models": ["qwen/qwen3-coder:free", "nvidia/nemotron-3-super-120b-a12b:free"],
  "route": "fallback"
}
```

When the RL engine has enough data, it can inject preferred models into the request, steering `openrouter/auto` toward known-good models for each task type.

### 6. Performance Dashboard

New section in the dashboard showing model performance:

```
Model Performance (last 30 days)
──────────────────────────────────────────────────
Model                          Success  Avg Score  Avg Fix  Cost
qwen/qwen3-coder:free            82%      87/100    0.4    $0.000
nvidia/nemotron-120b:free         76%      81/100    0.8    $0.000
deepseek/deepseek-r1              94%      93/100    0.2    $0.004
meta-llama/llama-4-maverick       91%      90/100    0.3    $0.001
```

### 7. Aggregation Queries

```sql
-- Best free model for code generation
SELECT model_id,
       AVG(analyze_score) as avg_score,
       SUM(success) * 100.0 / COUNT(*) as success_rate,
       AVG(fix_iterations) as avg_fixes
FROM model_performance
WHERE is_free = 1 AND task_type = 'code_generation'
GROUP BY model_id
ORDER BY success_rate DESC, avg_score DESC;

-- Free vs paid comparison
SELECT is_free,
       AVG(analyze_score),
       AVG(fix_iterations),
       SUM(success) * 100.0 / COUNT(*) as success_rate
FROM model_performance
GROUP BY is_free;
```

## Implementation

| Phase | Description | Status |
|-------|------------|--------|
| P1 | Ensure tool calls store actual model (not "openrouter/auto") | Pending |
| P2 | Add `model_performance` table to hexflo-coordination WASM module | Pending |
| P3 | Record performance entry after each quality gate completion | Pending |
| P4 | Update RL reward signal with free model cost efficiency bonus | Pending |
| P5 | Inject RL-learned model preferences into openrouter/auto requests | Pending |
| P6 | Dashboard: model performance panel with success rates | Pending |

## Consequences

### Positive
- **RL learns from free models** — discovers which free models are reliable per task type
- **Zero-cost optimization** — the best free models get promoted, reducing spend to $0
- **Data-driven selection** — preferences backed by actual compile/test/analyze results, not assumptions
- **Transparent** — dashboard shows exactly how each model performs

### Negative
- **Cold start** — needs ~50 runs to build reliable statistics per model
- **Free model churn** — OpenRouter's free model pool changes; old data may not apply
- **Storage** — one row per inference call per session adds up

### Mitigations
- Cold start: use the hand-picked defaults until RL has enough data
- Churn: prune entries older than 30 days; model_id captures the exact version
- Storage: aggregate after 30 days, keep only per-model summaries

## References

- ADR-2603232005: Self-Sufficient hex-agent with TUI
- ADR-2603231600: OpenRouter Inference Integration
- ADR-031: RL-Driven Model Selection & Token Management
- ADR-2603232230: Tool Call Tracking in SpacetimeDB
- [OpenRouter Auto-Router Docs](https://openrouter.ai/docs/guides/routing/routers/auto-router)

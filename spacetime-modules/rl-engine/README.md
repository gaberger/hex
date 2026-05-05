# rl-engine

> Q-learning bandit for model + context-strategy selection (ADR-2604102200).

A reinforcement-learning module that picks the best model action (`model:<name>`) and context strategy (`context:<strategy>`) for a given `state_key` (a hash of task type + features). Uses ε-greedy exploration with exploration bonuses for under-observed OpenRouter models, plus a `LOCAL_SUCCESS_BONUS` to bias the agent toward local Ollama models when they succeed.

## Tables

| Table | Visibility | Key | Purpose |
|---|---|---|---|
| `rl_experience` | public | `id` (unique) | Append-only log of (state, action, reward, next_state, task_type, ts) |
| `rl_q_entry` | public | `composite_id` (`{state}::{action}`) | Q-value table — q_value, visit_count, last_updated |
| `rl_pattern` | public | `id` (unique) | Distilled patterns — category, content, confidence, decay_rate, access_count |
| `rl_last_action` | public | `id` (unique) | Last action selected per agent/run (for reward attribution) |

## Hyperparameters (constants in `lib.rs`)

| Name | Value | Meaning |
|---|---|---|
| `EPSILON` | `0.1` | ε-greedy exploration rate |
| `LEARNING_RATE` | `0.1` | Q-update step size |
| `DISCOUNT_FACTOR` | `0.95` | Future-reward discount |
| `LOCAL_SUCCESS_BONUS` | `+0.1` | Reward bump for local models on success |
| `RATE_LIMIT_PENALTY` | `−0.5` | Reward penalty when rate-limited |
| `RATE_LIMIT_ALT_BOOST` | `+0.1` | Reward bump for alternates when one is rate-limited |
| `MAX_OPENROUTER_ENTRIES_PER_STATE` | `50` | Cap on OpenRouter Q-entries per state |
| `OPENROUTER_EXPLORATION_THRESHOLD` | `5` | Visits below this → exploration bonus applied |
| `OPENROUTER_EXPLORATION_BONUS` | `+0.15` | Bonus added to effective Q-value |
| `OPENROUTER_STALE_DAYS` | `30` | Caller-defined cutoff for `prune_stale_openrouter` |

## Action namespaces

- `model:<name>` — model-selection action (e.g. `model:qwen3:4b`, `model:openrouter:google/gemini-2.0-flash`).
- `context:<strategy>` — context-shaping action (e.g. `context:minimal`, `context:balanced`, `context:full`).

`select_action` returns a compound `model:...|context:...` choice via log.

### Tiered model defaults (seeded by `seed_model_q_values`)

| Tier | Models | Q-seed |
|---|---|---|
| 1 — Local fast | `nemotron-mini`, `qwen3:4b` | 0.25–0.30 |
| 2 — Local medium | `qwen3:8b`, `qwen3.5:9b` | 0.35–0.40 |
| 3 — Local coding | `qwen2.5-coder:32b`, `devstral-small-2:24b` | 0.45–0.50 |
| 4 — Cloud fallback | `sonnet`, `haiku`, `opus`, `minimax`, `minimax_fast` | 0.30–0.50 |

## Reducers

| Reducer | Args | Effect |
|---|---|---|
| `select_action` | `state_key` | Pick best (model, context) compound action; logs result, writes `rl_last_action` |
| `record_reward` | `state_key, action, reward, next_state_key, task_type, timestamp` | Append `rl_experience`, update `rl_q_entry` (Q ← Q + α(r + γ·max-next − Q)), apply local-success / rate-limit bonuses |
| `record_rate_limit` | `state_key, action, timestamp` | Apply `RATE_LIMIT_PENALTY` to `action`, boost siblings by `RATE_LIMIT_ALT_BOOST` |
| `prune_stale_openrouter` | `cutoff_timestamp` | Delete OpenRouter Q-entries with `last_updated < cutoff` (also enforces per-state cap) |
| `seed_model_q_values` | `state_key` | Insert default Q-entries from `MODEL_ACTIONS` table |
| `store_pattern` | `id, category, content, confidence` | Insert a distilled pattern |
| `decay_patterns` | — | Apply `decay_rate` to every pattern's confidence; deletes patterns below threshold |

## Subscriptions

```sql
SELECT * FROM rl_q_entry WHERE state_key = ?
SELECT * FROM rl_pattern WHERE category = ? ORDER BY confidence DESC
SELECT * FROM rl_experience ORDER BY timestamp DESC LIMIT 100
```

## Wiring

The `rl-engine` module is consulted by the dispatch path in `inference-gateway` (or by hex-nexus's tier router) before each request. The dispatcher calls `select_action`, observes the outcome, then calls `record_reward` or `record_rate_limit`. Patterns (`rl_pattern`) are surfaced to agents via the briefing buffer in `hexflo-coordination`.

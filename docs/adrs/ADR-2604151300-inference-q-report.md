# ADR-2604151300 — `hex inference q-report` CLI

**Status:** Proposed
**Date:** 2026-04-15
**Related:** ADR-2604120202 (tiered inference routing), ADR-2604131238 (`hex inference bench`), ADR-2604102200 (local-success bonus), ADR-2604151200 (idle research swarm — Q-report is a planned input signal)

## Context

The `rl-engine` SpacetimeDB module learns a Q-table over `(state, action)` pairs where state encodes `tier|task_type` and action is a model identifier. Reducers record rewards (`record_reward`) and select actions epsilon-greedily (`select_action`). The Q-table is documented in `docs/INFERENCE.md` with sample output:

```
tier:T1|rename_variable    model:qwen3:4b            Q=+1.308  visits=3
tier:T2|single_function    model:qwen2.5-coder:32b   Q=+0.110  visits=2
```

…but there is **no CLI command to actually print this table**. The doc shows what the data looks like; the operator has no way to see what their own system has learned. That makes it impossible to:

- Verify the router is converging
- Spot a model that's quietly degrading (Q dropping, visits flat)
- Compare local vs frontier Q-values to justify keeping or dropping a tier
- Feed the Q-table into the idle-research swarm (ADR-2604151200) as a code-quality signal

## Decision

Add `hex inference q-report` as a new subcommand under the existing `hex inference` group. It queries the `rl_q_entry` table via the SpacetimeDB client, joins relevant `rl_experience` rows for recency, and renders a human-readable report.

### Subcommand surface

```
hex inference q-report                             # Default: top entries by visits, all tiers
hex inference q-report --tier T1                   # Filter by tier
hex inference q-report --task-type fix_typo        # Filter by task type
hex inference q-report --model qwen3:4b            # Filter by model
hex inference q-report --sort q|visits|recency     # Sort key (default: visits)
hex inference q-report --limit 20                  # Row cap
hex inference q-report --format table|json|yaml    # Output format (default: table)
hex inference q-report --since 7d                  # Only entries with experience in window
hex inference q-report --watch                     # Live tail via STDB subscription
```

### Output (default `table` format)

```
hex inference q-report — 12 entries, last update 2m ago

  STATE                              MODEL                      Q       VISITS  LAST_SEEN  TREND
  tier:T1|fix_typo                   qwen3:4b                  +1.308    47     2m         ↑
  tier:T1|rename_variable            qwen3:4b                  +1.250    31     8m         ↑
  tier:T2|single_function            qwen2.5-coder:32b         +0.110    19     1h         →
  tier:T2|function_w_tests           qwen2.5-coder:32b         +0.092    14     3h         →
  tier:T2.5|multi_fn_cli             devstral-small-2:24b      +0.110    8      6h         ↑
  tier:T2|single_function            llama3.1:8b               -0.412    3      2d         ↓ (deprecated?)
  …

  Convergence: T1 stable (5 entries, ε-greedy fully exploiting)
  Drift:       1 entry trending down — `llama3.1:8b` for tier:T2|single_function
  Coverage:    9 of 12 documented task_types have at least 1 visit
```

The trend column compares current Q against the 7-day-ago Q from `rl_experience`. The footer is a 3-line summary intended to be greppable by the idle-research swarm and useful at a glance.

### Data path

`hex-cli` already talks to SpacetimeDB through hex-nexus REST endpoints for other queries. Add:

- `GET /api/inference/q-report?tier=&task_type=&model=&since=&limit=&sort=` — returns JSON array of Q-entries with computed `trend` field.
- `GET /api/inference/q-report/stream` — Server-Sent Events / WebSocket for `--watch`.

Both endpoints subscribe to `rl_q_entry` and `rl_experience` via the hex-nexus SpacetimeDB client (already present for other tables). The CLI is a thin renderer.

### Why not just exec a SQL query?

The data lives in SpacetimeDB tables, not SQLite. A direct subscription is correct, but the CLI shouldn't carry a SpacetimeDB client of its own — that's hex-nexus's job (ADR boundary: WASM-bridge concerns stay in hex-nexus). REST keeps the CLI thin and lets the dashboard reuse the same endpoint.

## Consequences

**Positive.**
- Operators can finally inspect what the router has learned. Trust grows with visibility.
- Provides a clean signal source for the idle-research swarm (ADR-2604151200) to flag drift.
- `--watch` mode makes the convergence story tangible during demos.

**Negative.**
- Adds two REST endpoints + a CLI subcommand surface to maintain.
- `--watch` opens a long-lived connection — needs cancellation handling, but hex-nexus already does this for other live tables.

## Non-goals

- Not changing the Q-learning algorithm itself.
- Not exposing reward-recording from the CLI (that path is internal to dispatch).
- Not adding write-side commands like `q-reset` — wipe paths are dangerous and belong in a separate ADR if ever needed.

## Implementation

See `wp-inference-q-report.json`.

# neural-lab

> Evolutionary neural architecture search — config tracking, experiment lifecycle, research frontiers, and UCB1-weighted mutation strategy.

Tracks candidate network configurations, runs them as `experiment` rows, and maintains a per-lineage `research_frontier` of the best configurations seen so far. Mutation strategies are picked via UCB1 (exploration bonus + observed success rate). Floats are stored as strings to avoid precision drift in SpacetimeDB.

## Tables

| Table | Visibility | Key | Purpose |
|---|---|---|---|
| `network_config` | public | `id` (unique) | Architecture hyperparameters — `n_layer`, `n_head`, `n_embd`, optimizer config, `parent_id` (lineage) |
| `layer_spec` | public | `id` (unique) | Per-layer attention type, window size, value embeddings, `resid_lambda`, `x0_lambda` |
| `experiment` | public | `id` (unique) | Training run — config_id, swarm_id, status, val_bpb, baseline_bpb, improvement_bpb, tokens, wall_time, lineage_name |
| `research_frontier` | public | `id` (unique) | Best config per lineage — `best_config_id`, `best_val_bpb`, totals (kept/discarded) |
| `mutation_strategy` | public | `id` (unique) | UCB1 selection — `selection_weight`, `total_tried`, `total_kept`, `success_rate` |
| `neural_lab_schedule` | public | `id` (PK) | Scheduler anchor for `research_loop_tick` |

## Constants

| Name | Value |
|---|---|
| `MIN_QUEUE_DEPTH` | `3` (per-lineage queue floor before auto-mutating) |
| `EXPLORATION_BONUS` | `1.0` (UCB1) |
| `DEFAULT_STRATEGIES` | `widen` · `deepen` · `attention` · `optimizer` · `activation` · `random` |

## Status values

- `network_config.status`: `candidate` · `active` · `archived`
- `experiment.status`: `queued` · `training` · `kept` · `discarded` · `failed`
- `layer_spec.attention_type`: `sliding_window` · `global` · `local` · `linear`

## Reducers

### Config + layers

| Reducer | Args | Effect |
|---|---|---|
| `config_create` | `name, parent_id, n_layer, n_head, n_kv_head, n_embd, vocab_size, sequence_len, window_pattern, activation, optimizer_config, total_batch_size, time_budget_secs, created_by` | Insert config (status=`candidate`); validates `n_layer > 0`, `n_embd > 0`, `vocab_size ≥ 256`, `time_budget_secs ≥ 60` |
| `config_activate` | `config_id` | Status → `active` |
| `config_archive` | `config_id` | Status → `archived` |
| `layer_spec_create` | `config_id, layer_index, attention_type, window_size, use_value_embeddings, resid_lambda, x0_lambda` | Insert per-layer spec |

### Experiments

| Reducer | Args | Effect |
|---|---|---|
| `experiment_create` | `config_id, swarm_id, hypothesis, mutation_diff, baseline_bpb, lineage_name` | Insert (status=`queued`) |
| `experiment_start` | `experiment_id, gpu_node_id, git_branch, git_commit, started_at` | Status → `training` |
| `experiment_complete` | `experiment_id, val_bpb, train_loss_final, tokens_processed, wall_time_secs, completed_at` | Status → `kept` or `discarded` based on `val_bpb < baseline_bpb`; updates `research_frontier` if best |
| `experiment_fail` | `experiment_id, error_message, completed_at` | Status → `failed` |

### Research loop (scheduled)

| Reducer | Args | Effect |
|---|---|---|
| `init` | — | Initial schedule registration (called by SpacetimeDB on module init) |
| `research_loop_tick` | — (scheduled) | Per-tick housekeeping — top up mutation queue per lineage |
| `stale_experiment_cleanup` | — (scheduled) | Mark long-running experiments as failed |
| `frontier_consolidate` | — (scheduled) | Recompute `research_frontier` totals |

### Mutation strategy

| Reducer | Args | Effect |
|---|---|---|
| `mutation_strategy_init` | — | Seed `DEFAULT_STRATEGIES` |
| `mutation_strategy_update` | — | Recompute UCB1 weights from `total_tried` / `total_kept` |

## Subscriptions

```sql
SELECT * FROM experiment WHERE lineage_name = ? ORDER BY started_at DESC
SELECT * FROM research_frontier ORDER BY best_val_bpb ASC
SELECT * FROM mutation_strategy ORDER BY selection_weight DESC
SELECT * FROM network_config WHERE status = 'active'
```

## Lineage model

`network_config.parent_id` links to the parent config that was mutated to produce this one. `lineage_name` (also on `experiment`) groups configs sharing a research direction. `research_frontier` is keyed by lineage — one frontier per lineage tracks the best `val_bpb` and counts.

## Float-as-string convention

`val_bpb`, `baseline_bpb`, `improvement_bpb`, `train_loss_final`, `selection_weight`, `success_rate`, `resid_lambda`, `x0_lambda` are all stored as strings. Callers are responsible for serializing/parsing as floats on the client side.

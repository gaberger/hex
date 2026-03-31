# ADR-2603241230: Neural Network Encoding in SpacetimeDB WASM

**Status:** Accepted
**Date:** 2026-03-24
**Drivers:** Research initiative to encode neural network architecture, weights, and autonomous experiment loops as SpacetimeDB transactional state — inspired by karpathy/autoresearch methodology
**Supersedes:** None (extends ADR-031 RL-Driven Model Selection)

<!-- ID format: YYMMDDHHMM — use your local time. 2603241230 = 2026-03-24 12:30 -->

## Context

[karpathy/autoresearch](https://github.com/karpathy/autoresearch) demonstrates that a neural network can be treated as **manipulable data**: a config dataclass (7 scalars) fully determines model architecture, a single-file training loop runs time-budgeted experiments, and an autonomous agent mutates config → trains → evaluates → keeps/discards in a tight loop. The key insight is that the "meta-program" (experiment orchestration) is separate from the "program" (training code).

hex already has SpacetimeDB WASM modules for RL (Q-learning via `rl-engine`), inference routing (`inference-gateway`), and swarm coordination (`hexflo-coordination`). However, there is no facility to:

1. **Encode a neural network's architecture as transactional state** — config, layer topology, hyperparameters as SpacetimeDB tables
2. **Track experiment history** with keep/discard branching and val_bpb metrics
3. **Coordinate autonomous research loops** where multiple agents propose architecture mutations, external GPU nodes execute training, and results feed back into the state atomically
4. **Observe experiments in real-time** via WebSocket subscriptions (dashboard integration)

### Forces

- **WASM sandbox constraint**: SpacetimeDB modules cannot access filesystem, network, or GPU. Training MUST execute externally (hex-nexus, fleet nodes, or cloud GPU). The module stores *descriptions* and *results*, not compute.
- **Autoresearch's fixed time-budget**: All experiments run for exactly 5 minutes wall-clock, making val_bpb directly comparable across architectural changes. This is the canonical evaluation protocol.
- **Autoresearch's single-metric simplicity**: `val_bpb` (validation bits per byte) is vocab-size-independent and normalizes across tokenizer changes. We adopt this as the primary fitness metric.
- **Existing rl-engine**: Already provides Q-learning for action selection (model routing). Can be extended to select architecture mutations via epsilon-greedy exploration.
- **Multi-agent research**: Unlike autoresearch (single agent), hex can coordinate a swarm of researcher agents — each proposing different mutations in parallel, with a supervisor selecting the best.

### Alternatives Considered

| Alternative | Why Rejected |
|-------------|-------------|
| Store NN state in SQLite only | No real-time subscriptions; no multi-agent coordination via WebSocket |
| Use external MLflow/W&B | Adds external dependency; doesn't integrate with hex swarm coordination |
| Encode weights as WASM tables | Weight tensors are too large for SpacetimeDB rows; store config + pointers instead |
| Single monolithic module | Violates per-module database separation (ADR-2603231500) |

## Decision

We will create a new SpacetimeDB WASM module `neural-lab` that encodes neural network architecture, experiment lifecycle, and autonomous research coordination as transactional tables and reducers. Training execution remains external; the module is the **state authority** for what to train, what was tried, and what worked.

### 1. Network Architecture as Tables

The neural network is encoded as a **config record** (inspired by autoresearch's `GPTConfig` dataclass) plus a **layer topology graph**:

```
┌──────────────────────────────────────────────────────────────┐
│  NetworkConfig (1 row per architecture variant)              │
│  ─────────────────────────────────────────────────────────── │
│  id: String (UUID)                                           │
│  name: String                                                │
│  parent_id: String (branching lineage)                       │
│  n_layer: u32                                                │
│  n_head: u32                                                 │
│  n_kv_head: u32                                              │
│  n_embd: u32                                                 │
│  vocab_size: u32                                             │
│  sequence_len: u32                                           │
│  window_pattern: String ("SSSL")                             │
│  activation: String ("relu_squared" | "gelu" | "swiglu")     │
│  optimizer_config: String (JSON: lr, warmup, muon params)    │
│  total_batch_size: u32                                       │
│  time_budget_secs: u32 (default: 300)                        │
│  created_at: String (ISO 8601)                               │
│  created_by: String (agent_id or "human")                    │
│  status: String ("candidate" | "active" | "archived")        │
└──────────────────────────────────────────────────────────────┘

┌──────────────────────────────────────────────────────────────┐
│  LayerSpec (optional per-layer overrides)                     │
│  ─────────────────────────────────────────────────────────── │
│  id: String                                                  │
│  config_id: String (FK → NetworkConfig)                      │
│  layer_index: u32                                            │
│  attention_type: String ("full" | "sliding_window")          │
│  window_size: u32                                            │
│  use_value_embeddings: String ("true" | "false")             │
│  resid_lambda: String (float as string for precision)        │
│  x0_lambda: String                                           │
└──────────────────────────────────────────────────────────────┘
```

### 2. Experiment Lifecycle (Keep/Discard Protocol)

Following autoresearch's branching model, each experiment is an atomic unit:

```
┌──────────────────────────────────────────────────────────────┐
│  Experiment                                                  │
│  ─────────────────────────────────────────────────────────── │
│  id: String (UUID)                                           │
│  config_id: String (FK → NetworkConfig)                      │
│  swarm_id: String (FK → hexflo swarm, nullable)              │
│  hypothesis: String (what change is being tested)            │
│  mutation_diff: String (JSON: what changed from parent)      │
│  status: String ("queued"|"training"|"evaluating"|           │
│                   "kept"|"discarded"|"failed")               │
│  val_bpb: String (float as string; null until evaluated)     │
│  baseline_bpb: String (parent config's best val_bpb)         │
│  improvement_bpb: String (baseline - val_bpb; positive=good) │
│  train_loss_final: String                                    │
│  tokens_processed: u64                                       │
│  wall_time_secs: u32                                         │
│  gpu_node_id: String (which fleet node ran this)             │
│  git_branch: String (experiment branch name)                 │
│  git_commit: String (commit hash of train.py at eval time)   │
│  started_at: String                                          │
│  completed_at: String                                        │
│  error_message: String (if status=failed)                    │
└──────────────────────────────────────────────────────────────┘

┌──────────────────────────────────────────────────────────────┐
│  ResearchFrontier (the "best known" for each lineage)        │
│  ─────────────────────────────────────────────────────────── │
│  id: String                                                  │
│  lineage_name: String ("gpt-small", "gpt-medium", etc.)     │
│  best_config_id: String (FK → NetworkConfig)                 │
│  best_experiment_id: String (FK → Experiment)                │
│  best_val_bpb: String                                        │
│  total_experiments: u32                                      │
│  total_kept: u32                                             │
│  total_discarded: u32                                        │
│  updated_at: String                                          │
└──────────────────────────────────────────────────────────────┘
```

### 3. Autonomous Research Loop (Reducer Protocol)

The experiment lifecycle follows autoresearch's keep/discard branching:

```
  Agent proposes mutation
         │
         ▼
  ┌─────────────────┐
  │ experiment_create│  reducer: validates config, sets status="queued"
  └────────┬────────┘
           │
           ▼
  ┌─────────────────┐
  │ experiment_start │  reducer: sets status="training", records gpu_node_id
  └────────┬────────┘  (called by hex-nexus when GPU node picks up job)
           │
           ▼
  ┌─────────────────┐
  │ experiment_eval  │  reducer: sets status="evaluating", records val_bpb
  └────────┬────────┘  (called by GPU node after training completes)
           │
     ┌─────┴─────┐
     │ improved?  │  compare val_bpb < baseline_bpb
     └─────┬─────┘
       ┌───┴───┐
       ▼       ▼
   "kept"   "discarded"
       │       │
       ▼       │
  Update       │
  Frontier     │
       │       │
       └───┬───┘
           ▼
    Next experiment
    (agent proposes
     new mutation)
```

**Reducers (external-triggered):**

- `experiment_create(config_id, hypothesis, mutation_diff)` → inserts Experiment with status="queued"
- `experiment_start(experiment_id, gpu_node_id)` → status="training", started_at=now
- `experiment_complete(experiment_id, val_bpb, train_loss, tokens, wall_time, git_commit)` → computes improvement, sets status="kept" or "discarded", updates ResearchFrontier if improved
- `experiment_fail(experiment_id, error_message)` → status="failed"
- `config_create(parent_id, ...fields)` → creates new NetworkConfig with lineage
- `frontier_query(lineage_name)` → returns best config + recent experiments

**Scheduled Reducers (server-side autonomous loop):**

SpacetimeDB scheduled reducers run inside the WASM module on a timer — no external agent needed for orchestration. This is the "meta-program" running server-side:

- `research_loop_tick()` — **scheduled every 30 seconds**. The core autonomous loop:
  1. Scan for experiments with status="completed" → compute keep/discard verdict, update frontier
  2. Scan for experiments with status="training" + wall_time > time_budget × 1.5 → mark as timed_out
  3. If queued experiments < `MIN_QUEUE_DEPTH` (default 3), auto-generate mutation candidates from the current frontier config using deterministic strategies (rotate through: widen, deepen, attention, optimizer, activation)
  4. Emit `ExperimentQueued` event for hex-nexus to pick up via subscription

- `stale_experiment_cleanup()` — **scheduled every 5 minutes**. Reclaims experiments stuck in "training" for > 2× time_budget (GPU node crash recovery).

- `frontier_consolidate()` — **scheduled every 10 minutes**. Recomputes ResearchFrontier from all "kept" experiments (consistency repair after crashes).

- `mutation_strategy_update()` — **scheduled every 15 minutes**. Reads rl-engine Q-values to adjust mutation strategy weights. Pure table reads + writes — no I/O needed.

**What scheduled reducers CAN do (pure computation inside WASM):**
- Read/write all tables in the module's database
- Perform math: compute val_bpb deltas, statistical aggregations, moving averages
- Generate deterministic mutation candidates (e.g., "increase n_embd by 128" based on current config)
- Cross-reference rl-engine tables (if same database) or use local strategy tables
- Emit log entries for dashboard consumption

**What scheduled reducers CANNOT do (WASM sandbox limits):**
- Execute training (no GPU, no PyTorch, no filesystem)
- Fetch external data (no HTTP, no network)
- Load/save weight checkpoints (no filesystem)
- Call hex-nexus REST API (no outbound connections)

**Bridge pattern:** hex-nexus subscribes to the `Experiment` table via WebSocket. When `research_loop_tick()` inserts a new experiment with status="queued", hex-nexus sees it instantly and dispatches the training job to a GPU node. When the GPU node finishes, it calls `experiment_complete` reducer via hex-nexus → SpacetimeDB. The loop is closed without any polling.

```
┌──────────────────────────────────────────────────────────┐
│  SpacetimeDB WASM (neural-lab module)                    │
│                                                          │
│  research_loop_tick() ──[30s]──→ insert Experiment       │
│  mutation_strategy_update() ──[15m]──→ read rl Q-values  │
│  frontier_consolidate() ──[10m]──→ recompute best        │
│                                                          │
│  Tables: NetworkConfig, Experiment, ResearchFrontier,    │
│          MutationStrategy, ExperimentLog                 │
└──────────────────────┬───────────────────────────────────┘
                       │ WebSocket subscription
                       ▼
┌──────────────────────────────────────────────────────────┐
│  hex-nexus (filesystem bridge)                           │
│                                                          │
│  Subscribes to Experiment table                          │
│  status="queued" → dispatch to GPU fleet                 │
│  GPU result → call experiment_complete reducer           │
│  Weight checkpoint → save to ~/.hex/neural-lab/          │
└──────────────────────────────────────────────────────────┘
```

### 4. Multi-Agent Research Swarm

Unlike autoresearch's single-agent loop, hex enables parallel research via HexFlo:

```
┌─────────────────────────────────────────────────────┐
│  ResearchSwarm (extends hexflo swarm)               │
│                                                     │
│  Supervisor Agent                                   │
│  ├── reads ResearchFrontier                         │
│  ├── proposes N mutation candidates                 │
│  ├── creates N experiments (queued)                 │
│  └── dispatches to GPU fleet                        │
│                                                     │
│  Mutator Agents (N parallel)                        │
│  ├── each gets a mutation strategy:                 │
│  │   - "widen" (increase n_embd)                    │
│  │   - "deepen" (increase n_layer)                  │
│  │   - "attention" (change window_pattern)          │
│  │   - "optimizer" (adjust lr, warmup, Muon params) │
│  │   - "activation" (try relu² vs swiglu)           │
│  │   - "random" (epsilon-greedy from rl-engine)     │
│  └── generate train.py + config via code mutation   │
│                                                     │
│  Evaluator Agent                                    │
│  ├── collects completed experiments                 │
│  ├── calls experiment_complete reducer              │
│  ├── updates frontier                               │
│  └── feeds reward signal to rl-engine               │
│                                                     │
│  Dashboard (real-time via WebSocket)                 │
│  ├── experiment history chart (val_bpb over time)   │
│  ├── architecture lineage tree                      │
│  └── fleet GPU utilization                          │
└─────────────────────────────────────────────────────┘
```

### 5. Weight Storage Strategy

Neural network weights are NOT stored in SpacetimeDB tables (too large — even a small GPT is ~100MB). Instead:

- **Config + metadata** → SpacetimeDB tables (small, transactional, real-time)
- **Weight checkpoints** → filesystem via hex-nexus, referenced by path in Experiment table
- **Best weights per frontier** → archived by hex-nexus to `~/.hex/neural-lab/checkpoints/{lineage}/{experiment_id}.pt`
- **Weight diffs** (for kept experiments) → optional delta-compression for storage efficiency

### 6. Integration with Existing Modules

| Module | Integration |
|--------|------------|
| `rl-engine` | Mutation strategy selection: state=current_config_hash, action=mutation_type, reward=improvement_bpb |
| `inference-gateway` | Trained models registered as inference providers for hex agents |
| `hexflo-coordination` | Research swarm lifecycle, task assignment to GPU nodes |
| `fleet-state` | GPU node registry, availability, capability (VRAM, compute) |
| `agent-registry` | Researcher agent lifecycle + heartbeats |

## Consequences

**Positive:**
- Neural network architecture becomes **first-class transactional state** — any mutation is atomic, observable, and auditable
- **Scheduled reducers run the autonomous loop server-side** — no external agent needed to orchestrate experiments; the WASM module IS the meta-program
- Real-time dashboard visibility into experiment progress via WebSocket subscriptions
- Multi-agent parallel research (N mutations simultaneously) — linear speedup over autoresearch's serial loop
- Full lineage tracking — every architecture change traced to parent, agent, hypothesis
- RL-driven mutation selection improves over time (rl-engine learns which mutations work)
- Integrates with existing hex infrastructure (HexFlo swarms, fleet nodes, agent registry)
- **Event-driven, not polling:** hex-nexus subscribes to table changes — zero-latency dispatch when new experiments are queued

**Negative:**
- WASM cannot execute training — requires external GPU nodes or hex-nexus bridge for actual compute
- SpacetimeDB string-encoded floats add serialization overhead (no native f32/f64 precision guarantees)
- Weight checkpoints stored outside SpacetimeDB break the "single source of truth" pattern
- Scheduled reducers cannot cross module boundaries — if rl-engine is a separate database (ADR-2603231500), strategy reads require hex-nexus to bridge the data
- Complexity: adds a new module + dashboard views + fleet GPU coordination

**Mitigations:**
- hex-nexus already bridges WASM↔filesystem; extend with GPU job dispatch queue
- Use string-encoded floats consistently (same pattern as inference-gateway) with explicit precision in val_bpb comparisons
- Weight checkpoint paths stored in Experiment table maintain referential integrity; hex-nexus cleanup job garbage-collects discarded experiment weights
- For cross-module data (rl-engine Q-values): hex-nexus periodically syncs a `MutationStrategy` table inside neural-lab with Q-values from rl-engine, keeping the scheduled reducer self-contained
- Phased implementation — start with config tables + experiment tracking, add multi-agent and dashboard later

## Implementation

| Phase | Description | Status |
|-------|------------|--------|
| P1 | Create `neural-lab` WASM module with NetworkConfig, Experiment, ResearchFrontier tables + core reducers | Pending |
| P2 | hex-nexus REST endpoints: `/api/neural-lab/experiments`, `/api/neural-lab/configs`, `/api/neural-lab/frontier` | Pending |
| P3 | CLI commands: `hex neural-lab experiment create/start/complete`, `hex neural-lab frontier` | Pending |
| P4 | Single-agent research loop: supervisor agent that proposes mutations serially (autoresearch parity) | Pending |
| P5 | Integration with rl-engine: mutation strategy as RL action space | Pending |
| P6 | Multi-agent research swarm via HexFlo: parallel mutations with supervisor coordination | Pending |
| P7 | Dashboard views: experiment timeline, lineage tree, frontier leaderboard | Pending |
| P8 | Fleet GPU dispatch: training jobs routed to available compute nodes | Pending |
| P9 | Weight checkpoint management: archival, delta-compression, garbage collection | Pending |

## References

- [karpathy/autoresearch](https://github.com/karpathy/autoresearch) — Autonomous AI research loop methodology
- ADR-031: RL-Driven Model Selection & Token Budget Management
- ADR-030: Multi-Provider Inference Broker
- ADR-025: SpacetimeDB as Distributed State Backend
- ADR-027: HexFlo Native Swarm Coordination
- ADR-035: Hex Architecture v2: Rust-First, SpacetimeDB-Native

## Implementation Notes

Implemented in:
- `hex-nexus/src/neural_lab_quant.rs` — quantization-aware neural lab runtime bridging WASM↔filesystem
- `spacetime-modules/` — neural-lab WASM module with NetworkConfig, Experiment, ResearchFrontier tables and scheduled reducers
- `hex-cli/src/commands/` — `hex neural-lab` CLI subcommands (experiment create/list, loop start/stop, frontier, config)
- ADR-2603231500: SpacetimeDB Per-Module Databases
- Autoresearch key metrics: 5-min time budget, val_bpb (bits per byte), MuonAdamW optimizer, keep/discard branching

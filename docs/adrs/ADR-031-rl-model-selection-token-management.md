# ADR-031: RL-Driven Model Selection & Token Budget Management

**Status:** Accepted
**Implementation-Present:** 2026-05-12 by auto-scan — evidence: hex-agent/src/adapters/secondary/rl_client.rs, hex-agent/src/ports/rl.rs, hex-agent/src/usecases/conversation.rs
**Date:** 2026-03-18
**Deciders:** Gary
**Relates to:** ADR-024 (hex-nexus), ADR-028 (API Optimization), ADR-030 (Multi-Provider)

## Context

hex-agent interfaces with multiple LLM providers (Anthropic, MiniMax, local) across models of varying quality, speed, and cost. Choosing the right model for each request is a multi-objective optimization problem:

- **Quality**: Opus > Sonnet > MiniMax M2.5 > Haiku > Local
- **Cost**: Local(free) < MiniMax($0.30/$1.20) < Haiku($0.80/$4) < Sonnet($3/$15) < Opus($15/$75)
- **Speed**: Haiku < MiniMax-Lightning < Sonnet < MiniMax < Opus
- **Rate limits**: Each model has independent RPM, input TPM, output TPM limits
- **Context window**: 200k (Anthropic), 192k (MiniMax) — partitioned across system/history/tools

Static routing (always use Sonnet) wastes money on simple tasks and hits rate limits on complex swarms. Manual model selection doesn't scale across 10+ concurrent agents.

## Decision

Use a reinforcement learning (RL) engine hosted in hex-nexus to learn optimal model selection and context strategy per task type. The agent queries the RL engine before each API call, and reports rewards after.

### 1. State Space

The RL state is a discretized representation of the current task context:

```rust
pub struct RlState {
    pub task_type: String,      // "conversation", "code_analysis", "summarization"
    pub codebase_size: u64,     // project size bucket
    pub agent_count: u8,        // concurrent agents in swarm
    pub token_usage: u64,       // cumulative tokens this session
    pub rate_limited: bool,     // was the last request 429'd
    pub retry_count: u8,        // consecutive retries this session
    pub current_model: String,  // model currently in use
}
```

State keys are discretized for the Q-table: `"conversation:sz0:ag1:tk2"` where `tk` buckets are `0..1k`, `1k..10k`, `10k..50k`, `50k..200k`, `200k+`.

### 2. Action Space

Actions are compound strings encoding both model selection and context strategy:

```
"model:opus|context:aggressive"
"model:sonnet|context:balanced"
"model:minimax|context:conservative"
"model:haiku"                        // defaults to balanced
"context:conservative"               // defaults to sonnet
```

#### Model Selection

```rust
pub enum ModelSelection {
    Opus,           // claude-opus-4-6 — highest quality, highest cost
    Sonnet,         // claude-sonnet-4-6 — balanced (default)
    Haiku,          // claude-haiku-4-5 — fast/cheap classification
    MiniMax,        // MiniMax-M2.5 — near-Opus quality, 10x cheaper
    MiniMaxFast,    // MiniMax-M2.5-Lightning — 2x speed
    Local,          // ollama/vllm — zero cost, no rate limits
}
```

#### Context Strategy

```rust
pub enum ContextStrategy {
    Aggressive,     // history×1.3, tools×1.2 — maximize context
    Balanced,       // history×1.0, tools×1.0 — default partitions
    Conservative,   // history×0.7, tools×0.8 — preserve budget
}
```

### 3. Reward Function

After each conversation turn, the agent computes a scalar reward:

```
reward = success(+1.0)
       + fast_bonus(if latency < 2s: +0.2)
       - rate_limit_penalty(if 429: -0.5)
       - token_cost(tokens_used / 10000)
       - tool_penalty(if max_rounds hit: -0.5)
```

Additional signals reported:
- `rate_limited: bool` — whether any 429 occurred during the turn
- `model_used: String` — actual model (may differ from selected if fallback occurred)
- `latency_ms: u64` — end-to-end response time

Rate-limited models get an immediate `-0.5` reward at the moment of the 429, before the fallback chain executes. This teaches the RL engine to avoid models that are near their limits.

### 4. Fallback Chain

When a model returns 429, the conversation loop tries the next model in the chain:

```
Opus → Sonnet → MiniMax → MiniMaxFast → Haiku → Local → (error)
```

Fallback is disabled when the user pins a model via `--model` (the `model_pinned` flag).

### 5. Token Budget Partitioning

The context window is divided into zones to prevent starvation:

```rust
pub struct TokenBudget {
    max_context: u32,           // e.g. 200,000
    response_reserve: u32,      // 15% reserved for output
    partitions: TokenPartition {
        system_fraction: 0.15,  // CLAUDE.md, skills, agents
        history_fraction: 0.40, // conversation turns
        tool_fraction: 0.30,    // tool_use results
    }
}
```

The RL-selected `ContextStrategy` modifies these fractions at runtime:
- **Aggressive**: expands history and tool budgets for complex reasoning
- **Conservative**: shrinks them to leave headroom for long sessions

### 6. Auto-Compaction (ADR-029 integration)

When context utilization exceeds `compact_threshold` (default 85%), the conversation loop automatically:
1. Summarizes the current conversation into a checkpoint
2. Clears message history
3. Optionally injects a new system prompt
4. Generates a new conversation ID

This prevents the context window from filling up and forcing truncation.

### 7. Model-Pinning Override

Users can bypass RL with `--model`:
```bash
hex-agent --model claude-opus-4-6    # Always use Opus, ignore RL
hex-agent --provider minimax          # Always use MiniMax
hex-agent                             # RL decides (default)
```

When pinned, the RL engine still receives state queries (for context strategy) but model selection is ignored.

## Architecture

```
                    ┌──────────────┐
                    │  RL Engine   │ (hex-nexus, Q-learning)
                    │  /api/rl/*   │
                    └──────┬───────┘
                           │ HTTP
                    ┌──────┴───────┐
                    │  RlPort      │ (port)
                    │  select_action│
                    │  report_reward│
                    └──────┬───────┘
                           │
              ┌────────────┴────────────┐
              │    ConversationLoop      │ (usecase)
              │                         │
              │  1. Query RL → action   │
              │  2. Adjust budget       │
              │  3. Check rate limiter  │
              │  4. Send to LLM        │
              │  5. Record metrics      │
              │  6. Report reward       │
              └──┬──────────┬───────┬──┘
                 │          │       │
         ┌───────┘    ┌─────┘   ┌──┘
         ▼            ▼         ▼
   ┌──────────┐ ┌──────────┐ ┌──────────┐
   │Anthropic │ │ OpenAI   │ │  Noop    │
   │Adapter   │ │ Compat   │ │  RL      │
   │(Opus/    │ │(MiniMax/ │ │(default  │
   │Sonnet/   │ │Together/ │ │strategy) │
   │Haiku)    │ │Ollama)   │ │          │
   └──────────┘ └──────────┘ └──────────┘
```

## Port Contract

```rust
#[async_trait]
pub trait RlPort: Send + Sync {
    /// Query the RL engine for the optimal action given current state.
    async fn select_action(&self, state: &RlState) -> Result<RlAction, RlError>;

    /// Report a reward to the RL engine after task completion.
    async fn report_reward(&self, reward: &RlReward) -> Result<(), RlError>;
}
```

Two adapters:
- `RlClientAdapter` — HTTP client to hex-nexus's `/api/rl/action` and `/api/rl/reward`
- `NoopRlAdapter` — returns `context:balanced` with Sonnet, used when hub is unavailable

## Files

| File | Layer | Role |
|------|-------|------|
| `ports/rl.rs` | Port | RlPort trait, RlState, RlAction, RlReward, ModelSelection, ContextStrategy |
| `adapters/secondary/rl_client.rs` | Adapter | HTTP client to hub RL engine |
| `usecases/conversation.rs` | Usecase | Integrates RL query → API call → reward reporting |
| `domain/tokens.rs` | Domain | TokenBudget, TokenPartition, TokenUsage |
| `domain/api_optimization.rs` | Domain | RateLimitState, CacheMetrics, WorkloadClass |

## Consequences

### Positive
- **Self-optimizing**: RL learns per-project, per-task optimal model selection over time
- **Rate limit resilience**: 429s train the engine away from saturated models
- **Cost awareness**: token_cost term in reward penalizes expensive models when cheaper ones suffice
- **Graceful degradation**: NoopRlAdapter provides sensible defaults when hub is unreachable
- **Observable**: all reward signals flow through the hub for dashboard visualization

### Negative
- **Cold start**: First few sessions use defaults until the Q-table has enough data
- **Hub dependency for RL**: Without hex-nexus, the agent always uses Sonnet/Balanced
- **Exploration noise**: RL may occasionally try suboptimal models (epsilon-greedy exploration)

### Risks
- Reward function weights may need tuning per deployment (high-budget orgs vs cost-sensitive teams)
- Q-table state space grows with number of task types — may need function approximation for large deployments
- Latency of the RL query (~5ms to hub) is negligible but adds a serial dependency before each API call

## Future Work

- **Thompson sampling** instead of epsilon-greedy for more efficient exploration
- **Per-agent RL state** in swarm mode (different agents learning different specializations)
- **Cost budget constraint**: hard cap on $/session with the RL engine staying within it
- **Bandit mode**: simplified multi-armed bandit for model selection only (no context strategy)
- **Offline training**: replay logged rewards to pre-train Q-tables for new projects

## References
- `hex-agent/src/ports/rl.rs` — Port definition with 12 unit tests
- `hex-agent/src/usecases/conversation.rs` — Integration point (reward formula at line ~350)
- `hex-agent/src/adapters/secondary/rl_client.rs` — HTTP adapter + NoopRlAdapter
- ADR-028: API Optimization Layer (rate limiting, caching that feeds RL signals)
- ADR-030: Multi-Provider Inference Broker (model variants the RL engine selects from)

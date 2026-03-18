# ADR-024: Hex-Hub Autonomous Nexus Architecture

## Status

Accepted

## Context

hex currently depends on Claude Code as the agent execution runtime. Claude Code owns the conversation loop, context window management, skill dispatch, hook execution, and agent spawning. hex-hub exists only as a passive dashboard — it observes but does not control.

This creates several problems:
1. **Vendor lock-in**: hex cannot function without Claude Code CLI installed
2. **No remote execution**: All work happens on the local machine
3. **No learning**: AgentDB RL patterns live in ruflo (now superseded by HexFlo, see ADR-027) (TypeScript/CLI), not in the orchestration layer
4. **No autonomous operation**: hex-hub cannot independently drive a plan/build/test cycle
5. **Context waste**: Claude Code's context window is shared with hex framework overhead

## Decision

Promote hex-hub from passive dashboard to **autonomous orchestration nexus**. Build a new **hex-agent** Rust binary that replaces Claude Code's role as the AI conversation runtime.

### Architecture Overview

```
┌─────────────────────────────────────────────────────┐
│                    HEX-HUB (Nexus)                  │
│  ┌──────────┐ ┌──────────┐ ┌───────────┐ ┌───────┐│
│  │ Chat UI  │ │ Workplan │ │ RL Engine │ │  SSH  ││
│  │ (WebSocket)│ │ Executor │ │ (AgentDB) │ │Deploy ││
│  └────┬─────┘ └────┬─────┘ └─────┬─────┘ └───┬───┘│
│       │             │             │            │    │
│  ┌────┴─────────────┴─────────────┴────────────┴───┐│
│  │           Orchestration Layer (Axum)             ││
│  │  Agent Lifecycle · Swarm Coord · Token Budget    ││
│  └──────────────────┬──────────────────────────────┘│
│                     │ spawn/monitor/stream           │
└─────────────────────┼───────────────────────────────┘
                      │
        ┌─────────────┼─────────────┐
        ▼             ▼             ▼
   ┌─────────┐  ┌─────────┐  ┌─────────┐
   │hex-agent│  │hex-agent│  │hex-agent│  (local or remote)
   │ (Rust)  │  │ (Rust)  │  │ (Rust)  │
   └────┬────┘  └────┬────┘  └────┬────┘
        │             │             │
   Anthropic     Anthropic     Anthropic
   Messages API  Messages API  Messages API
```

### hex-agent Binary (New)

A standalone Rust binary following hex architecture internally:

- **Domain**: Message, TokenBudget, ConversationState, ToolCall, AgentDefinition, Skill, Hook
- **Ports**: AnthropicPort, ContextManagerPort, SkillLoaderPort, HookRunnerPort, ToolExecutorPort
- **Secondary Adapters**: reqwest-based Anthropic client (SSE streaming), filesystem tools, token counter
- **Primary Adapters**: stdin/stdout CLI, WebSocket client (connects back to hex-hub)
- **Use Cases**: ConversationLoop (multi-turn with tool_use), ContextPacker (smart window management)

### AgentDB RL in hex-hub (Moved from ruflo, now HexFlo)

SQLite-backed reinforcement learning engine:
- **State space**: task type, codebase size, available agents, current token usage
- **Action space**: agent selection, context packing strategy, parallelism level, skill routing
- **Reward signal**: task completion (binary), token efficiency, time-to-completion, test pass rate
- **Algorithm**: Tabular Q-learning with epsilon-greedy exploration (simple, interpretable, sufficient for discrete action space)
- **Decay**: Temporal confidence decay on patterns (half-life: 7 days)

### Remote Compute (SSH)

hex-hub can SSH into compute nodes to:
1. Deploy hex-agent binary (scp)
2. Install HexFlo swarm coordinator
3. Kick off workplan phases remotely
4. Stream results back via SSH tunnel or reverse WebSocket
5. Health-check fleet with periodic heartbeats

### Context Window Manager

Token-aware context packing in hex-agent:
- **Budget partitions**: system prompt (15%), conversation history (40%), tool results (30%), response reserve (15%)
- **Eviction strategy**: Oldest turns first, but pin turns containing tool_use results referenced by later turns
- **Summarization**: When evicting, compress old turns into a summary block
- **AgentDB integration**: Query RL engine for optimal packing strategy per task type

## Consequences

### Positive
- hex operates independently of Claude Code
- Remote execution enables distributed builds
- RL loop learns from every interaction, improving orchestration over time
- Token budgets are managed precisely, not wasted on framework overhead
- Single Rust binary deploys anywhere (cross-compile)

### Negative
- Significant new Rust code (~5,000-8,000 LOC across hex-agent + hex-hub extensions)
- Must maintain parity with Anthropic API changes
- SSH deployment adds security surface (key management, network exposure)
- RL engine needs sufficient training data before outperforming heuristics

### Risks
- Anthropic API tool_use format may evolve — mitigate with adapter abstraction
- RL cold-start problem — mitigate with sensible defaults that RL refines
- Remote compute security — mitigate with SSH key-only auth, no password

## Dependencies

- ADR-015 (Hub SQLite Persistence) — RL tables extend existing schema
- ADR-016 (Hub Binary Version) — hex-agent version must also be verified
- ADR-011 (Multi-Instance Coordination) — remote agents coordinate via hub API
- ADR-022 (Coordination Wiring) — prerequisite for orchestration routes

## Implementation Plan

See `docs/workplans/feat-hex-nexus.json` for tier-ordered task decomposition.

### Phase 1: Foundation (Tier 0-1)
- hex-agent domain types + port traits
- Anthropic adapter with SSE streaming
- Context window manager
- Skill/hook/agent loaders

### Phase 2: Conversation Loop (Tier 2-3)
- Multi-turn tool_use loop
- CLI primary adapter
- Composition root wiring

### Phase 3: Hub Integration
- RL engine in hex-hub
- Chat UI with WebSocket
- Orchestration routes (agent lifecycle, workplan executor)

### Phase 4: Remote Compute
- SSH adapter (russh)
- Binary deployment pipeline
- Fleet management

### Phase 5: Integration
- End-to-end tests
- RL training with seed data
- Documentation

# ADR-2026-04-10-2200: RL-Driven Agent Infrastructure Self-Improvement

**Status:** Accepted
**Date:** 2026-04-10
**Drivers:** Session revealed agent infrastructure gaps during README rewrite attempt

## Context

### What Happened

Today we tried to use hex agents to rewrite README.md:
1. Created HexFlo swarm `readme-rewrite`
2. Called `hex task create` → returned empty `tasks: []`
3. Tried Agent tool → `ProviderModelNotFoundError`
4. Fell back to direct Write

### The Problem: No Agentic Brain

hex has pieces (HexFlo, agent workers, steering) but **no brain** that:
- Receives agentic requests
- Routes to appropriate handler
- Learns from outcome
- Adapts method selection

### The Vision: Agentic Brain

```
User Request: "rewrite the README"
       ↓
   Agentic Brain
       ↓
├─ Parse intent: documentation
├─ Check capabilities: [worker, task, agent tool]
├─ Try method 1: worker + task creation
│   ├─ Create swarm → 201 ✓
│   ├─ Create task → [] ✗ (learned: empty response)
│   └─ Score: failure → method downweighted
├─ Try method 2: Agent tool
│   ├─ Spawn → ProviderModelNotFoundError ✗
│   └─ Score: failure → method downweighted
├─ Try method 3: direct Write
│   ├─ Write → success ✓
│   └─ Score: success → method upweighted
└─ Return result with learning
```

## Decision

Implement **Agentic Brain** — a central orchestrator that handles all agentic requests with RL-driven adaptation.

### Architecture

```
┌─────────────────────────────────────────────────────┐
│                 Agentic Brain                       │
├─────────────────────────────────────────────────────┤
│  Intent Parser    → What does user want?            │
│  Capability Check → What's available right now?   │
│  Method Router  → Try best → fallback → fail       │
│  Outcome Tracker → Success/failure/incomplete    │
│  RL Learner    → Adapt based on outcomes         │
└─────────────────────────────────────────────────────┘
```

### P1: Intent Parser

Map user requests to handler types:
- `code` → hex-coder worker
- `documentation` → documenter worker  
- `test` → tester worker
- `review` → reviewer worker
- `agent` → Agent tool (if configured)
- `write_file` → direct Write (fallback)

### P2: Capability Probe

On any agentic request:
1. Check inference providers: `hex inference list`
2. Check workers: `hex agent list` 
3. Check steering: ping pause/resume baseline

Return what works, mark unavailable methods as blocked.

### P3: Method Scoring

Track per-session success per method:
```rust
struct MethodScore {
    method: String,      // "worker+task", "agent_tool", "direct_write"
    request_type: String, // "code", "doc", "test"
    attempts: u32,
    successes: u32,
    avg_latency_ms: f64,
}
```

RL updates: `score = success_rate / latency_ms`

### P5: Adaptive Routing

Use RL scores to pick method:
```
given request_type, available_methods:
  scores = method_scores[request_type].filter(|m| available.contains(m.method))
  if scores.is_empty():
    fallback = try_all_available_methods()
    return fallback.best()
  pick scores.max_by(|s| s.success_rate / s.latency_ms)
```

## Implementation

| Phase | Description | Validation | Status |
|-------|-------------|-------------|--------|
| P1 | Intent parser: map request → type | "code" → hex-coder | Pending |
| P2 | Capability probe: check what's working | Returns [worker, inference] | Pending |
| P3 | Method scoring: track success/failure per method | Metrics visible | Pending |
| P4 | Steering baseline: verify worker works before use | Worker pauses/resumes | Pending |
| P5 | Adaptive routing: pick best method via RL | Best score used | Pending |

## Consequences

**Positive:**
- Hex learns from its own failures → each session smarter
- No more silent fallbacks → clear errors
- Proactive capability checking → fail fast
- RL adaptation → better method selection over time

**Negative:**
- Startup latency for capability probe
- Storage for method scores
- Complexity in routing decision

**Mitigations:**
- Probe runs async (non-blocking)
- In-memory scoring (session scope)
- Simple scoring: success_rate / latency

## References

- ADR-2026-03-27-1000: Quantization-Aware Inference Routing (RL for inference)
- ADR-2026-04-10-2100: Agentic Steerable Loop (what we built today)
- This session: README rewrite attempt revealed gaps
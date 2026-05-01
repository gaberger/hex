# ADR-2604180001: Workplan Inference Task Stalling & State Sync

## Status
ACTIVE — Critical blocker for autonomous workplan execution

## Context

During e2e validation testing on Bazzite (2026-04-17), discovered that workplan execution processes hang silently when executing inference tasks:

1. **Workplan loads correctly**: 6 phases, 23 tasks
2. **Phase 1 initiates**: P1-1 task starts with qwen2.5-coder:32b (T2 tier)
3. **Process hangs**: No output, no progress, no error messages
4. **Accumulation**: Multiple processes spawn (5 observed) with no cleanup
5. **State loss**: hex-nexus reports "No active workplan executions" despite processes running
6. **Resource leak**: Processes consume memory/CPU indefinitely

## Problem Statement

The workplan executor cannot reliably track or manage long-running inference tasks. Specific failures:

- **Inference blocking**: Task execution stalls indefinitely on Ollama inference call
- **Timeout missing**: No timeout/heartbeat mechanism to detect stalls
- **State desync**: Running processes not reflected in nexus state
- **Orphan cleanup**: Stalled processes accumulate without being killed
- **Feedback lost**: No way to know what failed or why

This prevents autonomous workplan execution at scale (Phase 1 of 6 fails to complete).

## Decision

Implement comprehensive task execution monitoring with:

1. **Task-level timeouts** (per inference tier: T1 30s, T2 120s, T2.5 300s)
2. **Heartbeat mechanism** (emit progress every 30s, fail if silent >timeout)
3. **State sync bridge** (sync task progress to nexus in real-time)
4. **Orphan cleanup** (background reaper kills stalled tasks after timeout)
5. **Error reporting** (surface stall reason: timeout, OOM, network, inference error)

## Implementation

Phased approach:

### Phase 1: Immediate Diagnostics (Blocker Removal)
- [ ] Determine if stall is: inference hang, I/O hang, or CPU spin
- [ ] Log ALL task state transitions (queued → running → done/error)
- [ ] Implement task-level timeout guards with clear error messages
- [ ] Add heartbeat output every 30s during inference

### Phase 2: State Synchronization
- [ ] Sync task progress to nexus DB at each checkpoint
- [ ] Implement orphan detection (process exists but task state stale)
- [ ] Add task cleanup on stall timeout (SIGTERM → SIGKILL escalation)

### Phase 3: Resilience
- [ ] Implement task retry with exponential backoff
- [ ] Add circuit breaker for tier escalation (if T2 stalls, escalate to T2.5)
- [ ] Persist task state to disk for recovery across restarts

## Consequences

**Positive**:
- Workplans can complete autonomously
- Clear error messages instead of silent hangs
- Automatic cleanup of stalled tasks
- Observable progress during long inference calls

**Negative**:
- Adds latency to task execution (heartbeat overhead)
- More aggressive timeout may kill legitimate slow tasks
- Requires nexus connectivity for full feature set (local fallback less capable)

## Related

- ADR-2604120202 (Tiered inference routing)
- ADR-2603240130 (Declarative swarm from YAML)
- ADR-005 (Compile gate retry pipeline)
- wp-bazzite-e2e-arch-validation (blocked by this issue)

## Evidence

Session 2026-04-17:
- Workplan: `docs/workplans/wp-bazzite-e2e-arch-validation.json`
- Environment: Bazzite (Linux, GPU, SpacetimeDB online, Ollama online, hex-nexus online)
- Failure mode: 5 `hex plan execute` processes running, 0% CPU, no output, no state updates after 15+ minutes
- Stall trigger: Phase P1-1 inference task (qwen2.5-coder:32b T2)
- Root cause: TBD (need diagnostic logs to confirm inference hang vs. other blockage)

# ADR-022: Wire Coordination into Use Cases (Last-Mile Fix)

**Status**: Accepted
**Date**: 2026-03-17
**Deciders**: Core Team
**Related**: ADR-004 (Worktrees), ADR-011 (Coordination), ADR-020 (Feature UX)

## Context

ADR-011 specified a full multi-instance coordination system: worktree locks, task claiming, activity broadcasting, and unstaged file tracking. The port (`ICoordinationPort`), adapter (`CoordinationAdapter`), and hub endpoints are all implemented and tested.

However, **no use case calls coordination**. The three critical orchestration use cases — `WorkplanExecutor`, `SwarmOrchestrator`, and `FeatureProgressOrchestrator` — don't accept `ICoordinationPort` in their constructors and therefore can't acquire locks, claim tasks, or publish activity.

### Observed Failures

1. Multiple agents edit the same files simultaneously, causing lost writes
2. Two instances of `/hex-feature-dev` can spawn agents for the same task
3. No agent knows what files other agents are touching
4. Worktrees are created without lock protection

### Root Cause

The composition root creates a `CoordinationAdapter` and stores it in `AppContext.coordination`, but the use case constructors never receive it. Classic "last mile" wiring gap.

## Decision

Wire `ICoordinationPort` into the three orchestration use cases and enforce lock-before-worktree, claim-before-spawn semantics.

### 1. Inject Coordination into Use Case Constructors

```typescript
// WorkplanExecutor
constructor(
  private readonly swarm: ISwarmPort,
  private readonly fs: IFileSystemPort,
  private readonly coordination: ICoordinationPort | null,  // NEW
) {}

// SwarmOrchestrator
constructor(
  private readonly swarm: ISwarmPort,
  private readonly worktree: IWorktreePort,
  private readonly coordination: ICoordinationPort | null,  // NEW
) {}

// FeatureProgressOrchestrator
constructor(
  private readonly progress: IFeatureProgressPort,
  private readonly coordination: ICoordinationPort | null,  // NEW
) {}
```

Coordination is `| null` because hex-hub may not be running — use cases must degrade gracefully.

### 2. Lock Before Worktree Creation

In `SwarmOrchestrator`, before creating a worktree:

```typescript
if (this.coordination) {
  const lock = await this.coordination.acquireLock(feature, layer);
  if (!lock.acquired) {
    throw new WorktreeConflictError(feature, layer, lock.holder);
  }
}
const worktree = await this.worktree.create(branchName);
```

### 3. Claim Before Agent Spawn

In `WorkplanExecutor`, before spawning an agent:

```typescript
if (this.coordination) {
  const claim = await this.coordination.claimTask(task.id);
  if (!claim.claimed) {
    throw new TaskConflictError(task.id, claim.holder);
  }
}
await this.swarm.spawnAgent(agentId, role, task.id);
```

### 4. Publish Activity on Key Events

Use cases publish to the activity stream at:
- Worktree creation/deletion
- Agent spawn/completion
- Task start/finish
- Phase transitions

### 5. Pass Context to Spawned Agents

Before spawning, query active agents and pass context:

```typescript
const unstaged = await this.coordination.getUnstagedAcrossInstances();
const activities = await this.coordination.getActivities(10);
// Pass as agent context via swarm memory
await this.swarm.memoryStore(`agent-${agentId}-context`, JSON.stringify({
  lockedFiles: unstaged,
  activeAgents: activities,
  ownedLock: lock,
}));
```

### 6. Release on Completion

```typescript
// In finally block
if (this.coordination) {
  await this.coordination.releaseLock(lockId);
  await this.coordination.releaseTask(taskId);
}
```

## Graceful Degradation

When `coordination` is `null` (hub not running):
- Skip lock acquisition — behave as today (single-instance mode)
- Log a warning: "hex-hub not available, coordination disabled"
- Never block or throw

## Composition Root Changes

Update `composition-root.ts` to pass coordination when constructing use cases:

```typescript
const executor = new WorkplanExecutor(swarm, fs, appContext.coordination);
const orchestrator = new SwarmOrchestrator(swarm, worktree, appContext.coordination);
```

## Success Metrics

- Two `/hex-feature-dev` runs on the same project cannot claim the same task
- Worktree locks prevent concurrent edits on the same layer
- `hex status` shows which agent owns which files
- Zero lost writes from agent interference

## Testing Strategy

- Unit: Mock coordination port, verify lock/claim called before worktree/spawn
- Integration: Two concurrent WorkplanExecutors, second one gets conflict error
- Property: Random interleaving of lock/claim/release never deadlocks

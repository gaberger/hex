# ADR-2604142300: Brain Auto-Cleanup of Stale Swarms

**Status:** Accepted
**Date:** 2026-04-14
**Implemented:** 2026-04-14 (`hex-cli/src/commands/brain.rs::check_stale_swarms` + `autofix_stale_swarm`)
**Drivers:** Brain daemon auto-fixes stale binary + reconciles workplans, but swarms stay active forever even after their workplan completes. 9+ stale swarms accumulated unnoticed. Brain must own swarm lifecycle too.

## Context

Today brain validate checks:
- CLI wiring
- Binary freshness  
- Workplan status
- MCP-CLI parity
- Worktree health

Missing: **swarm lifecycle.** When a workplan reaches 100% task completion, its associated swarm should auto-complete. Currently:
- Workplan completes: ✓
- Swarm stays `active` with 0 tasks: ✗ — dead weight in `hex swarm status`

Operator discovers this manually (today's session: 9 stale swarms cleaned up).

## Decision

### 1. New validate check: `check_stale_swarms()`

Brain validate adds a 6th check:

```
Swarms: ✗ 3 stale swarms (workplan done but swarm active)
  - 61a70d3f wp-aios-experience-p1 (workplan 19/19 done, swarm active)
  - ...
```

### 2. Auto-fix (like binary rebuild + workplan reconcile)

Each tick:
1. Fetch all active swarms from `/api/swarms`
2. For each swarm, find associated workplan by name
3. If workplan status is `done` (all tasks complete): call `swarm_complete(swarm_id)`
4. Log: "auto-completed stale swarm {id} ({name})"

### 3. Safety

- Only auto-complete if ALL tasks `status: done` AND workplan file has `status: done|completed`
- Never auto-complete if any task is `pending` or `in_progress`
- Audit entry logged to HexFlo memory for each auto-complete

## Consequences

**Positive:**
- `hex swarm status` always accurate
- Dashboard swarm count reflects reality
- No more operator-initiated swarm sweeps

**Negative:**
- One more API call per tick
- Risk of premature completion if workplan status is wrong

**Mitigations:**
- Completion requires BOTH task-level and workplan-level confirmation
- Dry-run mode via env var `HEX_BRAIN_DRY_RUN=1`

## Implementation

| Phase | Description | Status |
|-------|------------|--------|
| P1 | Add `check_stale_swarms()` function to brain.rs | Done |
| P2 | Wire into validate() output + daemon auto-fix | Done |
| P3 | Dry-run mode via `HEX_BRAIN_DRY_RUN=1` | Done |
| P4 | Audit logging to HexFlo memory | Pending |

## References

- ADR-2604131945: Brain Self-Consistency Daemon
- ADR-2604132300: Brain Daemon Loop
- ADR-027: HexFlo Swarm Coordination

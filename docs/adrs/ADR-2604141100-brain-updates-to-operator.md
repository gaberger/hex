# ADR-2604141100: Brain Updates to Operator — Push, Don't Poll

**Status:** Proposed
**Date:** 2026-04-14
**Drivers:** Brain daemon runs autonomously but operator has no visibility unless they manually check `hex brain queue list`. An AIOS should keep the operator informed without being asked.

## Context

Today brain daemon:
- Ticks every N seconds
- Validates, drains queue, executes tasks
- Marks tasks completed/failed
- Writes `brain_tick` events to SpacetimeDB

But NONE of this surfaces to the operator unless they run a query. "Fire and forget" shouldn't mean "fire and forget about it" — operator needs to know when:
- A task completes (especially if it found issues)
- A task fails
- Brain validate finds new problems (regression)
- Workplan execution finishes
- Architecture grade drops

## Decision

### 1. Brain posts to inbox after every meaningful event

Extend brain daemon to call `hex inbox notify` (ADR-060) after:
- **Task completion** — priority 1 (info), body = summary
- **Task failure** — priority 2 (urgent), body = error
- **Validate regression** — priority 2 (when NEW issues appear vs last tick)
- **Workplan completion** — priority 1, body = phases/tasks summary
- **Architecture grade change** — priority 2 if grade drops below A

### 2. `hex brain watch` — live stream

```bash
hex brain watch          # Tails brain_tick events via WebSocket
hex brain watch --since 1h   # Replay last hour
```

Subscribes to SpacetimeDB `brain_tick` table changes and prints them as they happen.

### 3. Inbox summary on `hex` (no args)

Already shows status. Add: "Brain: N new notifications since last check. Run `hex inbox` to review."

### 4. Auto-notify on session start

`hex hook session-start` already runs. Add: query `hex inbox` for unacked brain notifications, print count + top 3.

### 5. Configurable verbosity

```toml
# .hex/daemon.toml
[notifications]
on_task_complete = true      # info-level
on_task_failure = true       # urgent
on_validate_regression = true
on_workplan_complete = true
on_grade_drop = true
min_priority = 1             # Filter below this
```

## Consequences

**Positive:**
- Operator is informed, not ignorant
- Regressions surface immediately
- `hex brain watch` gives live feedback
- Session start shows what happened while you were away

**Negative:**
- More inbox traffic — risk of noise
- Extra REST calls per tick (~100ms overhead)

**Mitigations:**
- Priority filter prevents noise
- Batch notifications per tick (one inbox entry per tick, not per task)
- Opt-out via config for users who prefer silence

## Implementation

| Phase | Description | Status |
|-------|------------|--------|
| P1 | Brain daemon POSTs to /api/inbox after task completion | Pending |
| P2 | Detect validate regressions (diff vs last tick) | Pending |
| P3 | `hex brain watch` streaming command | Pending |
| P4 | Session-start inbox summary | Pending |
| P5 | Config file support | Pending |

## References

- ADR-060: Agent Notification Inbox
- ADR-2604132300: Brain Daemon Loop
- ADR-2604132330: Brain Inbox Queue
- ADR-2604140000: Hey Hex

# ADR-2604132300: Brain Daemon — The Missing Supervisor Loop

**Status:** Proposed
**Date:** 2026-04-13
**Drivers:** hex has a nexus daemon, SpacetimeDB, cleanup services, and agent pollers — but NO supervisor loop that continuously validates, auto-fixes, and advances project state. Without it, hex is a collection of tools, not an operating system. An AIOS needs an init process.

## Context

Today hex has several isolated loops:
- **hex-nexus**: long-running HTTP server, has a cleanup service on 60s interval
- **hex-agent worker**: polls for tasks every 5s
- **SpacetimeDB reducers**: event-driven on table changes

But nothing runs `hex brain validate` continuously. Nothing auto-reconciles workplans. Nothing detects stale binaries without human prompt. The user has to manually run `hex go` to trigger checks — defeating the ambient-AIOS premise.

### Comparison

| System | Supervisor | Autonomous? |
|--------|-----------|-------------|
| Unix | init/systemd | Yes — starts/restarts services, watches health |
| Kubernetes | kubelet + controllers | Yes — reconciles desired vs actual state continuously |
| Ralph loops (Claude) | self-iterating agent | Yes — runs until goal or max-iterations |
| Claude cronjobs | external scheduler | Yes — at interval |
| **hex today** | **nothing** | **No** — requires manual triggers |

## Decision

### 1. `hex brain daemon` — the supervisor loop

```bash
hex brain daemon              # Start the supervisor (foreground)
hex brain daemon --background # Start detached, PID file
hex brain daemon stop         # Stop the daemon
hex brain daemon status       # Is it running?
```

### 2. What the loop does each tick (default 60s)

| Check | Auto-fix | Action on fail |
|-------|----------|----------------|
| Nexus running | Yes — restart | Log + notify |
| Release binary fresh | Yes — rebuild | Skip if cargo busy |
| Workplans consistent | Yes — reconcile | Log drift |
| Worktrees clean | Yes — cleanup merged | Log stale >24h |
| Tests compile | No (suggest only) | Log failures |
| Architecture grade | No (suggest only) | Log if dropped below A |
| Pending workplan tasks | No (alert only) | Log ready-to-execute tasks |

### 3. Configurable interval + backoff

```toml
# .hex/daemon.toml
[daemon]
interval_secs = 60
max_consecutive_failures = 3
backoff_on_failure = true

[checks]
binary_freshness = true
workplan_reconcile = true
worktree_cleanup = true
architecture_grade = true
```

### 4. Event emission to SpacetimeDB

Each tick writes a `brain_tick` row with:
- timestamp
- checks run
- issues found
- auto-fixes applied
- duration

Dashboard subscribes to `brain_tick` for live supervisor status.

### 5. Integration with existing loops

- Reuses `HexFlo::cleanup_stale_agents()` (nexus cleanup service)
- Triggers `hex agent worker` polling if tasks queued
- Coordinates with SpacetimeDB reducers (doesn't duplicate work)

### 6. Safety — never block, never loop forever

- Each check has a hard timeout (5s)
- Max consecutive failures before pause (3)
- `ctrl-C` or `hex brain daemon stop` exits cleanly
- PID file prevents multiple instances

## Consequences

**Positive:**
- hex becomes a true AIOS — continuously supervising
- No more "is hex healthy?" — the daemon knows
- Dashboard gets live health stream
- `hex go` becomes optional (daemon does it automatically)
- Pairs naturally with SpacetimeDB event model

**Negative:**
- One more process to manage
- CPU overhead (minimal — 60s interval)
- Potential for feedback loops if auto-fixes introduce new issues

**Mitigations:**
- Daemon respects `max_consecutive_failures` to prevent runaway fixes
- All auto-fixes are idempotent
- Daemon is opt-in (not required for hex to work)

## Implementation

| Phase | Description | Status |
|-------|------------|--------|
| P1 | `hex brain daemon` command with foreground loop + interval | Pending |
| P2 | Tick writes `brain_tick` to SpacetimeDB | Pending |
| P3 | Background mode with PID file + stop command | Pending |
| P4 | Config file support + per-check toggles | Pending |
| P5 | Dashboard integration (live brain_tick stream) | Pending |

## References

- ADR-2604131945: Brain Self-Consistency Daemon (validate checks)
- ADR-2604131500: AIOS Developer Experience (ambient UX)
- Claude cronjobs + Ralph loops (inspirations)
- Unix init(1), Kubernetes kubelet (supervisor patterns)

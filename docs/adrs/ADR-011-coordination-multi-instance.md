# ADR-011: Coordination and Multi-Instance Locking

## Status: Accepted
## Date

2026-03-17

## Context

hex runs multiple Claude Code sessions in parallel, each operating in its own git worktree. Without coordination, two agents may claim the same task, write to the same worktree, or silently overwrite each other's unstaged changes. ADR-004 introduced worktrees for filesystem isolation, but it assumed a single orchestrator process. In practice, a developer may launch multiple Claude Code windows, or a swarm coordinator may spawn agents that outlive the parent session.

We need a system that:
- Prevents two agents from editing the same worktree layer simultaneously.
- Tracks which agent owns which task to avoid duplicate work.
- Publishes an activity stream so agents and developers can see what is happening across all instances.
- Detects stale instances whose processes have died without cleanup.

## Decision

Implement an `ICoordinationPort` with five capabilities: instance registration, worktree locking, task claiming, activity publishing, and unstaged file tracking. The port is backed by the hex-hub daemon (already running for the dashboard) via HTTP endpoints.

### Port Design

| Capability | Methods | Purpose |
|------------|---------|---------|
| **Instance lifecycle** | `registerInstance`, `heartbeat` | Each session registers on startup and sends heartbeats every 15s with its PID and unstaged files |
| **Worktree locks** | `acquireLock`, `releaseLock`, `listLocks` | Advisory locks keyed by `projectId:feature:layer`; prevents two agents from editing the same adapter boundary |
| **Task ownership** | `claimTask`, `releaseTask`, `listClaims` | First-come-first-served task claiming; returns conflict info if already claimed |
| **Activity stream** | `publishActivity`, `getActivities` | Append-only event log for cross-instance visibility |
| **Unstaged tracking** | `getUnstagedAcrossInstances` | Aggregates uncommitted changes from all instances, classified by hex layer |

### Why hex-hub Over Other Backends

| Approach | Pros | Cons |
|----------|------|------|
| **File-based locks** (`.lock` files) | No daemon needed | Race conditions on NFS; no heartbeat; no activity stream |
| **SQLite advisory locks** | Single-file DB, portable | Requires bundling SQLite; WAL mode conflicts with multiple writers |
| **Redis** | Battle-tested distributed locks | External dependency; overkill for single-machine dev tool |
| **hex-hub HTTP** (chosen) | Already running; authenticated; supports WebSocket for future push; in-memory state with persistence hooks | Requires hub daemon to be running |

hex-hub is already a required daemon for the dashboard (ADR-008). Adding coordination endpoints keeps the dependency count at zero -- no new processes or external services.

### Authentication

The hub daemon writes a `~/.hex/daemon/hub.lock` file containing an auth token. The `CoordinationAdapter` reads this token and includes it as a `Bearer` header on every request. This prevents unauthorized processes from manipulating locks.

### Heartbeat and Stale Detection

Each instance sends a heartbeat every 15 seconds. The heartbeat includes the current `git status --porcelain` output, classified by hex layer. The hub can detect stale instances by comparing `lastSeen` timestamps and optionally checking whether the registered PID is still alive.

### Lock Granularity

Locks are scoped to `projectId:feature:layer`, matching the worktree naming convention from ADR-004. This means two agents can work on the same feature if they target different layers (e.g., one on `primary-adapter`, another on `secondary-adapter`). This aligns with the dependency tier model where tiers 1 and 2 are independent.

## Consequences

### Positive

- Prevents duplicate work and conflicting writes across parallel Claude Code sessions.
- Activity stream gives developers real-time visibility without polling individual agents.
- Unstaged tracking enables a "what's in flight?" dashboard view across all instances.
- Advisory locking (not mandatory) means a single-agent workflow is unaffected -- coordination is opt-in.
- No new external dependencies; reuses the existing hex-hub daemon.

### Negative

- Requires hex-hub to be running for coordination features. Falls back gracefully (adapter methods return null/empty) when the hub is unavailable.
- In-memory state on the hub means locks are lost on hub restart. Acceptable because worktree locks are advisory and sessions re-register on startup.
- HTTP round-trip latency (~1-5ms per call) for lock acquisition. Acceptable given that lock operations happen at task boundaries, not in hot loops.

## Alternatives Considered

1. **File-based `.lock` files in each worktree** -- Rejected due to race conditions and no mechanism for heartbeat or stale detection.
2. **SQLite with WAL mode** -- Viable but adds a bundled dependency. Would require careful handling of concurrent writers from different processes.
3. **Redis or etcd** -- Rejected as over-engineered for a single-machine development tool. hex is not a distributed system.
4. **In-process mutex (no daemon)** -- Only works within a single process. Multiple Claude Code sessions are separate OS processes.

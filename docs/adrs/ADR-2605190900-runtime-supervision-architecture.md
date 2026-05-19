# ADR-2605190900: Runtime Supervision Architecture — closing the autonomous-loop liveness gap

**Status:** Proposed
**Date:** 2026-05-19
**Drivers:** A 2-hour live investigation of "the queue isn't draining" surfaced six independent failures that all share one missing layer: **the system has no architecture for component liveness**. The hexagonal substrate handles app-code structure cleanly, but there is no equivalent supervision layer governing the runtime processes that make the loop go. As long as that gap exists, every fix is whack-a-mole — patching the symptom under one component while four others are silently broken.

## Context

The autonomous loop has a chain of dependencies: **sched daemon → dispatcher → swarm-task → hex-agent worker → inference adapter → SpacetimeDB → workplan_executor → reconciler**. Each link is a separate process or port with its own lifecycle. None of them coordinate liveness with the others; each one assumes its dependencies are healthy.

On 2026-05-19, with the system **reporting itself as healthy** (`hex nexus status` → "running", `hex brain daemon-status` → "running pid=N", `hex doctor composition` → "Standalone variant, all services ✓"), six failures stacked invisibly:

| # | Component | Failure | How long invisible |
|---|---|---|---|
| 1 | `hex sched daemon` (pid 330376) | Zombie: stdout/stderr → `/dev/null`, log file frozen, not draining queue. Still reports itself "running pid=N" via the pid file. | **11 days** (since 2026-05-08) |
| 2 | Dispatcher (`dispatch_brain_task`) | Creates swarm-task under `brain-lease` swarm and waits for an agent to claim. No agent exists. Lease expires → daemon re-enqueues as fresh ID → infinite loop. | 30+ re-enqueue cycles observed in 30 min after restart |
| 3 | `hex-agent` worker | Process was supposed to be spawned by `hex nexus start`. Three `[hex-agent] <defunct>` zombies from 2026-05-18 in the process table, no live worker today. | ≥ 24 hours |
| 4 | `hex_agent` STDB table | Lists 3 agents (from May 7-8) all with `status=completed`. The "registered" list never deregistered on agent death and is essentially historical archaeology. | ≥ 11 days |
| 5 | STDB endpoint discovery | Nexus tries to reach `http://192.168.30.162:3033/v1/database/hex/sql` — a stale IP from `.hex/state.json` while STDB is actually at `127.0.0.1:3033`. Cache override silently wrong; no re-discovery on connection failure. | Unknown — surfaced when I queried |
| 6 | `workplan_auto_emitter` | Retries ADR-035 every 60 s with the same `no tool_calls in response` failure. ~30 burned LLM calls per hour, no backoff, no per-ADR error counter. | Indefinite — visible only in `nexus.log` grep |

Each is a different code path, but the pattern is identical: **the component fails alone, in silence, with no other component to flag the failure or take over.** The system is alive in the sense that the processes are running; it is **dead in the sense that no work is moving**. The current `hex nexus status` answers the wrong question — it says "is the process up" when the operator needs to know "is the chain end-to-end healthy".

## Decision

Add a **Runtime Supervision Layer** orthogonal to the application-code layers. It owns six concerns the current architecture leaves implicit:

### 1. Worker-pool invariant

The dispatcher publishes work to a logical pool; the pool guarantees a live consumer or fails fast.

- New STDB table `worker_pool { pool_id, role, min_workers, max_workers, last_heartbeat_at }`.
- New STDB table `worker_process { worker_id, pool_id, pid, host, registered_at, last_heartbeat_at, status }`.
- Dispatcher checks `count(worker_process WHERE pool_id=X AND last_heartbeat_at > now() - 30s) >= 1` before claiming a brain-task. If false → escalate to operator inbox immediately (Tier-B); never silently fan out into a void.

Today: `dispatch_brain_task` creates the swarm-task unconditionally. After: the dispatcher refuses to create work it cannot fulfill.

### 2. Heartbeat contract per port

Every long-running adapter (STDB, Ollama, `hex-agent` worker, `sched` daemon, `nexus`) registers a heartbeat with a known TTL.

- Each component calls `IHeartbeatPort::beat(role, status, evidence)` every N seconds.
- A `liveness_supervisor` STDB reducer ticks every 15 s, finds `(role, last_beat_at)` pairs older than `role.ttl`, and issues one **specific** action: `restart_local | escalate_operator | route_around`.
- No more "process up but doing nothing" — a stuck process stops beating, the supervisor sees it, the supervisor acts.

This subsumes today's `agent-registry` module but adds the **action-on-miss** behavior, which is the new bit.

### 3. Bounded retry with dead-letter

A brain-task that fails to drain N times within M minutes is **dead-lettered**, not silently re-enqueued.

- New STDB table `dead_letter { task_id, kind, payload, last_error, attempt_count, first_failed_at, last_failed_at }`.
- Dispatcher tracks `(task_id, attempt_count)`. On the Nth failure (default N=5 over 10 min), the task moves to `dead_letter` with a P1 inbox notification.
- Dashboard surface: `#/dead-letter` shows everything stuck, why, and the next op (manual replay, code-fix, cancel).

Today: invisible 60-second-period churn loops, no operator signal. After: an operator who looks at the dashboard sees the stuck items within 5 min.

### 4. Agent-process lifecycle ownership

A `pool_supervisor` STDB reducer owns spawn / heartbeat / reap for hex-agent workers, scoped per host.

- Reducer logic: "ensure exactly N live workers per pool". On startup, walks `worker_process` for this host, kills orphans (defunct pids, stale heartbeats), spawns missing.
- Worker process file descriptor management: stdout/stderr go to a known per-worker log file, never `/dev/null`. The first failure that leaves a zombie is itself recorded as an `improver_event`.
- Worker exit hook: deregisters from `worker_process` before exit. Crash hook (SIGCHLD on the supervisor) marks the row `crashed` and the supervisor decides whether to respawn.

This closes the bug that left three `[hex-agent] <defunct>` zombies pinned to `ppid=1` for 24+ hours.

### 5. STDB endpoint discovery

Single source of truth (`.hex/project.json` → `coordination.host`), re-validated on connection failure with a known fallback chain.

- On every connection error to STDB, the nexus discovery layer re-reads `project.json`, falls back to `localhost:3033`, then to a documented `HEX_STDB_FALLBACK_HOST` env var.
- `.hex/state.json` becomes read-only telemetry. The current behavior — state.json overrides project.json silently — is the bug.
- `hex doctor liveness` (item 6 below) gates green on STDB endpoint matching at least one of the documented hosts.

Today: nexus is talking to `192.168.30.162:3033` because some past session wrote that into `state.json` and nothing re-validated. After: connection failure triggers fresh discovery within one tick.

### 6. End-to-end liveness self-test

`hex doctor liveness` injects a synthetic ping-task and walks the full chain, reporting the **first** broken link.

- Synthetic task: `kind=ping`, `payload=<uuid>`, expects an `improver_event { kind: pong, related: <uuid> }` row within 60 s.
- Output: per-stage timestamp + status. The first stage that misses its deadline is the diagnosis.
- Required gate for dashboard's `Liveness: GREEN` badge. The current `hex nexus status` should not display ✓ unless `doctor liveness` is green.
- CI gate: a workflow job runs `hex doctor liveness` against an ephemeral nexus on every push. Today there is no such gate, so silent liveness regressions ship freely.

## Consequences

### Positive

- Operator sees a stuck component within one liveness-tick (~15 s) instead of 11 days.
- Dispatcher refuses to publish into a void; today's invisible 60-second-period churn becomes a P1 inbox event after 5 attempts.
- Zombie processes self-reap. Worker pools maintain target population. No more "I think it's running" / "no it's been dead for a week".
- STDB endpoint drift is impossible — connection failure forces re-discovery.
- A single `hex doctor liveness` answers "is the autonomous loop end-to-end healthy" instead of seven different commands that all answer "is this one process up".

### Cost

- New STDB tables (`worker_pool`, `worker_process`, `dead_letter`) and reducers in `hexflo-coordination`. Schema migration required.
- Every long-running component must implement the heartbeat trait. Backward-compat shim: a component that doesn't yet beat is treated as `legacy` and the supervisor only reaps, doesn't restart.
- An extra ~1 KB/s of STDB write traffic (heartbeats from ~10 components every 15 s × 50 B each).
- Dashboard surface grows by 2 panes (`#/liveness` + `#/dead-letter`).

### What this is NOT

This is **not** an attempt to make the existing dispatcher work. The existing dispatcher's contract — "fire and pray" — is exactly the bug. The redesign replaces the contract.

This is also **not** Kubernetes. Pool sizes are small (1–5 workers), the supervision is in-process (a STDB reducer), and there are no container abstractions. The complexity budget is "what an operator can hold in their head and the dashboard can show in one screen".

## Implementation

See `docs/workplans/wp-runtime-supervision.json` for phase decomposition. The 6 concerns above each get one or two phases. Phases are independently shippable; the order minimizes cross-phase dependencies:

1. **Heartbeat contract** (concern #2) — pure additive; existing components ignored.
2. **Dead-letter table + bounded retry** (#3) — replaces the silent re-enqueue loop. Single biggest signal jump for the operator.
3. **Pool supervisor + worker_process** (#1, #4) — reaps zombies, ensures consumer-exists-before-publish.
4. **STDB endpoint re-discovery** (#5) — defends against `state.json` drift.
5. **Liveness self-test** (#6) — composes everything above into one operator-facing verb.

Phases 1+2 alone resolve the symptoms observed today. Phases 3–5 close the class of bugs that produced them.

## References

- ADR-060 (Agent Notification Inbox) — P1/P2 priorities used by the dispatcher's escalation path.
- ADR-061 (Workplan Lifecycle Management) — the state model the dead-letter table extends.
- ADR-2605190720 (Evidence Gate) — the contract that already inverts "stored status" for workplans; this ADR applies the same inversion to runtime components.
- ADR-2605190721 (Self-Improvement Loop) — the MAPE-K loop this supervision layer makes actually possible. Without supervision the loop's discover/propose/judge phases run, but the act phase cannot land because no worker claims the resulting swarm-tasks. This ADR is the prerequisite.
- ADR-2026-04-26-1311 (Six-Layer Governance) — supervision is the runtime equivalent of governance: same idea, different time-scale.
- ADR-2026-04-13-2300 (Brain Daemon Loop) — the tick mechanism this ADR extends with action-on-miss semantics.
- Workplan: `docs/workplans/wp-runtime-supervision.json`.

## Postmortem evidence — 2026-05-19

Recorded so future operators can see the failure mode that motivated this ADR:

```
Hour 0       30 batches re-enqueued (3 workplans × 10 cycles, ~60 s period)
             0 commits, 0 drafts, 0 task-status mutations
             daemon "running pid=3801464" reported healthy throughout
             agent worker absent throughout
             3 defunct hex-agent processes still pinned to ppid=1
             nexus connecting to 192.168.30.162:3033 (wrong host)
             auto-emitter burning 1 LLM call/min on a deterministic failure
Hour 0+30m   2nd 15-min cron confirms STATE=RED, queue depth stable at 3
             same churn pattern, same root cause
             no part of the system noticed itself broken
Hour 2       operator opens an ADR. The system did not.
```

That last line is the ADR in one sentence: **the system was supposed to surface its own brokenness and didn't.** The supervision layer is what gives it that ability.

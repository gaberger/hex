# ADR-2604141400: Brain queue tasks require swarm ownership + confirmed completion

<!-- ID format: YYMMDDHHMM — 2604141400 = 2026-04-14 14:00 local -->

- **Status**: §1 P1 Accepted 2026-04-14 (git-evidence guard); §1 P2+ (swarm lease) + §2 still Proposed
- **Date**: 2026-04-14
- **Depends on**: ADR-2604132330 (brain queue), ADR-2604150000 (brain→sched rename), ADR-027 (HexFlo)
- **Relates to**: feedback_verify_before_done, feedback_use_hexflo_hex_agent

## Context

The brain daemon currently drains the queue by:

1. Pulling pending tasks off the queue
2. Running `execute_brain_task(kind, payload)` — a plain `std::process::Command` subprocess (see `hex-cli/src/commands/sched.rs:2010`)
3. Calling `update_brain_task(id, "completed" | "failed", stdout)` based on the subprocess exit code

This drain model has three failure modes all observed in practice:

1. **Subprocess-success ≠ work-actually-done.** `hex plan execute` exits 0 when the workplan's tasks are already marked `done` — even if the reconciler false-positively flipped them (based on file-path presence rather than end-to-end wiring). The task is marked completed with zero real work performed.
2. **No owner, no recovery.** If the subprocess hangs or the daemon restarts mid-task, there is no lease, no heartbeat, and no identity — the task is lost in an ambiguous state.
3. **No swarm integration.** HexFlo swarms (ADR-027) exist for coordinated multi-agent work, but brain tasks never become swarm tasks. Brain and HexFlo are parallel queues with no handshake.

The invariant the queue is *supposed* to enforce — "completed means the work is done" — is not enforced.

## Decision

### 1. Every dequeued task becomes an owned swarm task

When the brain daemon picks a task off the queue, it must:

1. Create (or attach to) a HexFlo swarm sized for the task kind
2. Register the brain task as a swarm task with `hexflo_task_id = brain_task.id`
3. Transition the brain task's status from `pending` to `leased` (new state), NOT `in_progress` directly, NOT straight to `completed`
4. Stamp `leased_to = <swarm_id>` and `leased_until = now + lease_duration`

The brain daemon never executes work itself — it is exclusively a dispatcher. HexFlo owns execution.

### 2. New task states: `pending → leased → in_progress → completed | failed`

| State | Transition | Who sets it |
|-------|------------|-------------|
| `pending` | enqueued | `hex brain enqueue` |
| `leased` | dispatched, swarm attached | brain daemon |
| `in_progress` | swarm agent started | swarm agent hook (`SubagentStart`) |
| `completed` | verification passed | confirm-complete check |
| `failed` | verification failed or lease expired | confirm-complete check / lease sweeper |

### 3. `completed` requires verification, not just subprocess exit

Before a brain task may transition to `completed`, the confirm-complete check must pass. Check depends on task kind:

| Kind | Confirmation |
|------|--------------|
| `workplan` | `hex plan reconcile <path>` reports all steps `done` with **git evidence** (not file-presence heuristic) — i.e., every step has a commit SHA in its `evidence.commits` array. If reconcile shows even one `pending` step post-run, the brain task transitions to `failed` with the pending step list as the failure reason. |
| `hex-command` | Subprocess exit 0 AND no stderr pattern matching known-failure strings (TBD, minimal initial set: `error:`, `panicked`, `FAILED`). |
| `shell` | Subprocess exit 0. (Unchanged — shell is already minimal scope.) |
| `remote-shell` (ADR-2604141200) | Agent PATCHed status back with exit 0. |

### 4. Lease expiry and re-enqueue

A dedicated sweeper runs every `lease_check_interval` (default: 30s). For every `leased` or `in_progress` task where `leased_until < now`:

- Emit `brain.lease_expired` event (priority 2 notification via ADR-2604141100)
- Release the swarm task (HexFlo `task_fail` with reason `lease_expired`)
- Return the brain task to `pending` with `lease_attempts += 1`
- After 3 failed lease attempts, mark `failed` permanently

Default lease duration by kind:

| Kind | Lease |
|------|-------|
| `workplan` | 30 min |
| `hex-command` | 5 min |
| `shell` | 2 min |
| `remote-shell` | 60 sec |

### 5. Storage shape

Extend the brain task value (currently stored in HexFlo memory `brain-task:{id}`):

```json
{
  "id": "...",
  "kind": "workplan",
  "payload": "...",
  "status": "leased",
  "leased_to": "<swarm_id>",
  "leased_until": "2026-04-14T18:30:00Z",
  "lease_attempts": 1,
  "swarm_task_id": "<hexflo_task_id>",
  "evidence": {
    "commits": [],
    "reconcile_verdict": null
  }
}
```

### 6. Reconciler hook must not flip brain tasks to `completed`

The pre-flight reconciler (auto-marking workplan steps `done` based on file-path matches) already exists and has produced false positives (see today's enqueue of 4 workplans where most tasks were flipped `done` without real implementation). With this ADR, **brain tasks are never updated by the reconciler** — only by the confirm-complete check inside the brain daemon. The reconciler may still annotate workplan task statuses *inside the JSON file*, but the brain task status is orthogonal.

## Consequences

### Positive

- Queue completions become meaningful. A `completed` brain task provably has git evidence.
- Failed swarms do not silently orphan work — the sweeper re-enqueues.
- HexFlo becomes the single execution substrate; brain shrinks to a scheduler.
- Lease telemetry feeds directly into `hex brain watch` and inbox notifications.

### Negative

- **Schema migration**: the `status` enum gains two variants (`leased`, `in_progress`). Older brain tasks pre-ADR will show as pending until re-processed — acceptable.
- **More round-trips per task**: enqueue → lease → in_progress → verify → complete. At 10s tick interval, this adds ~20-30s latency to simple tasks. Mitigation: shell + hex-command kinds skip the swarm dance (they are intentionally lightweight).
- **Reconciler + daemon coupling**: the confirm-complete check must read git and run `hex plan reconcile`. Adds dependency on nexus + git working tree cleanliness.

### Non-goals

- Multi-tenant lease coordination (assume single brain daemon per project; cluster brain is a future ADR).
- Retry policies for failed work (still 3-strikes-and-fail as today).

## Alternatives considered

- **Optimistic ack**: trust subprocess exit code and fix reconciler false positives instead. Rejected — reconciler bugs recur, and the "completed means done" invariant should not depend on every other subsystem behaving perfectly.
- **Move brain entirely into HexFlo**: obviates brain queue. Rejected for this ADR — too large a change; the hex-sched rename (ADR-2604150000) just landed and stability matters.

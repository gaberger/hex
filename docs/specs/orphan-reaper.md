# Orphan Reaper: Live-but-Unregistered Process GC

*status*: proposed  ·  *date*: 2026-05-21

Orphan Reaper: Live-but-Unregistered Process GC

**Implementation:** `hex-nexus/src/orchestration/orphan_reaper.rs` (commit 0f3bb916)  
**Sibling:** `hex-nexus/src/orchestration/zombie_sweeper.rs` (handles defunct processes)  
**Workplan:** wp-fix-workplan-inference-stalling task P4-1

## Purpose

The orphan reaper garbage-collects **live-but-unregistered** `hex-agent` daemon processes. These are worker processes still running on the host but no longer claimed by any `worker_process` row in STDB. This failure mode occurs in two scenarios:

1. **Nexus restart.** When hex-nexus dies, its in-process watchdog tasks (per-worker `tokio` tasks that poll `/proc/<pid>` every 2s) die with it. The `hex-agent` processes continue running. When the new nexus instance starts, it has no record of the old pids, reaps the stale `worker_process` rows, and spawns fresh workers. The orphaned processes remain alive indefinitely with no parent watcher and no signal to exit. Observed 2026-05-21: **94 hex-agent processes vs 32 worker_process rows** after a single restart cycle.

2. **Watchdog crash without process exit.** A panicking tokio task in nexus could lose the per-worker watcher without the hex-agent process actually terminating.

The orphan reaper is the **conservative** counterpart to zombie_sweeper: it targets processes that are **still alive** (not defunct), validates they match the hex-agent binary, confirms they have no claim in STDB, and escalates termination signals across two ticks (SIGTERM → SIGKILL).

---

## Algorithm

Runs every **60 seconds** (`REAP_INTERVAL_SECS`):

1. **Enumerate running hex-agent pids.** Walk `/proc`, collect every process where:
   - Executable basename (via `/proc/<pid>/exe` symlink) == `"hex-agent"`
   - Owner uid matches the current user (via `/proc/<pid>/status` `Uid:` line)
   - Process state ≠ `Z` (zombies are zombie_sweeper's domain)
   - Cmdline contains parseable `--agent-id <role>` arg
   - Age > **30 seconds** (`GRACE_PERIOD_SECS`) — protects newly-spawned workers that haven't been registered yet

2. **Fetch claimed pids from STDB.** Query `worker_process` table for rows with `status ∈ { 'healthy', 'degraded', 'starting' }`. Build a `Set<pid>` of claimed pids. If STDB is unreachable, **skip the tick** (safer to leave processes alone than kill them blind).

3. **Split running into orphans vs. claimed.** Any pid in the running set but NOT in the claimed set is an orphan.

4. **Escalation pass (second tick).** For each orphan pid that was SIGTERM'd in the **previous tick** and is still alive: send **SIGKILL**. Remove from escalation set after kill attempt.

5. **SIGTERM new orphans (first tick).** For each orphan pid not already in the escalation set: send **SIGTERM**. Add to escalation set for next tick. Log via `tracing::info!` with pid, role, age_secs.

6. **Audit log.** Best-effort write a summary to `IStatePort::hexflo_memory_store` under key `orphan_reap:<timestamp>` with JSON payload:
   ```json
   {
     "running_total": <int>,
     "claimed_total": <int>,
     "orphans_total": <int>,
     "sigterm": <int>,
     "sigkill": <int>
   }
   ```

---

## Behavioral Specifications

### `orphan_pid_sigtermed_on_first_tick`

**Given:**  
- A `hex-agent daemon --agent-id ceo` process with pid 12345 is running on the host  
- The process is owned by the nexus user (uid match)  
- Age > 30 seconds  
- No `worker_process` row in STDB has `pid=12345` with `status ∈ { 'healthy', 'degraded', 'starting' }`

**When:**  
- The orphan reaper tick executes

**Then:**  
- pid 12345 receives `SIGTERM` (via `libc::kill`)  
- A `tracing::info!` log line is emitted: `"orphan reaper: SIGTERM (no worker_process row claims this pid)"` with fields `pid=12345`, `role="ceo"`, `age_secs=<N>`  
- pid 12345 is added to the internal `pending_kill` set for escalation on the next tick

**Evidence:** `hex-nexus/src/orchestration/orphan_reaper.rs` lines 159–171 (SIGTERM pass)

---

### `orphan_pid_sigkilled_on_second_tick`

**Given:**  
- pid 12345 was SIGTERM'd in tick N and added to `pending_kill`  
- At tick N+1 (60 seconds later), pid 12345 is still running  
- pid 12345 is still NOT claimed by any `worker_process` row

**When:**  
- The orphan reaper tick N+1 executes

**Then:**  
- pid 12345 receives `SIGKILL` (via `libc::kill`)  
- A `tracing::warn!` log line is emitted: `"orphan reaper: SIGKILL (escalation after SIGTERM ignored)"` with fields `pid=12345`, `role="ceo"`, `age_secs=<M>`  
- pid 12345 is removed from the `pending_kill` set (no re-escalation on tick N+2)

**Evidence:** `hex-nexus/src/orchestration/orphan_reaper.rs` lines 142–154 (escalation pass)

---

### `fresh_spawn_skipped_in_grace_period`

**Given:**  
- A `hex-agent daemon --agent-id cfo` process with pid 67890 is running on the host  
- The process was spawned 15 seconds ago (age < 30s)  
- No `worker_process` row claims pid 67890

**When:**  
- The orphan reaper tick executes

**Then:**  
- pid 67890 is NOT enumerated in the `running` set (filtered out by `scan_hex_agent_processes`)  
- pid 67890 receives NO signals  
- No log line mentions pid 67890

**Rationale:** Newly-spawned hex-agents have a brief window between `fork()` and the `supervisor_subscriber` running `worker_process_register`. Without the grace period, the reaper would race the registrar and kill its own freshly-spawned workers.

**Evidence:** `hex-nexus/src/orchestration/orphan_reaper.rs` lines 293–298 (grace period filter in `scan_hex_agent_processes`)

---

## Implementation Notes

- **Conservative by default.** Only reaps pids that (1) match the `hex-agent` executable basename, (2) have a parseable `--agent-id` arg, (3) are owned by the current user, (4) are older than 30s, and (5) are NOT in state `Z`.

- **STDB failure = skip tick.** If the `fetch_claimed_pids` query fails (HTTP error, timeout, unreachable), the reaper returns `Err` and does NOT signal any processes. Safer to leave orphans alive than kill claimed workers blind.

- **Escalation state persists across ticks.** The `pending_kill` HashSet is owned by the `run()` loop and passed mutably to each `tick()` call. Pids move from "fresh orphan" → `pending_kill` (SIGTERM sent) → escalated (SIGKILL sent) → removed.

- **Audit log uses `hexflo_memory_store`.** Same pattern as zombie_sweeper. When `IStatePort` grows a typed `supervisor_event_insert` method, this becomes a direct call.

- **Spawn site:** `hex-nexus/src/lib.rs` (or wherever the supervisor + zombie_sweeper are spawned). Example:
  ```rust
  crate::orchestration::orphan_reaper::OrphanReaper::spawn(state.clone());
  ```

---

## Success Criteria

1. After a nexus restart, the number of live `hex-agent` processes on the host converges to the number of `worker_process` rows with `status ∈ { 'healthy', 'degraded', 'starting' }` within **120 seconds** (two ticks: SIGTERM + SIGKILL).

2. Newly-spawned workers (age < 30s) are never signaled, even if their registration is delayed by supervisor_subscriber backlog.

3. `tracing` logs clearly distinguish first-tick SIGTERM (info level) from second-tick SIGKILL (warn level) and include pid, role, and age_secs for operator correlation.

4. STDB query failure results in a skipped tick (no signals sent) and a `warn!` log entry.

---

## Related Work

- **zombie_sweeper.rs:** Detects **defunct** (state=`Z`) processes and logs them via `supervisor_event`. Does NOT send signals (zombies can only be reaped by their parent or init).

- **wp-fix-workplan-inference-stalling P4-1:** Task that motivated the orphan reaper implementation after observing 94 orphan hex-agent processes post-restart.

- **ADR-2026-05-19-0900 P3.3:** Introduced zombie_sweeper. Orphan reaper is the live-process complement.

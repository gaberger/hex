# ADR-2605211400 — child-reap-on-spawn-local-agent

Status: **Proposed**
Date: 2026-05-21

## Context

On 2026-05-21 13:09, a production nexus instance accumulated **4,544 defunct hex-agent processes** (all `ppid=current-nexus`, state `Z`). The delta over 20 seconds was zero — the leak is not currently active, but the zombie count is symptomatic of a contract gap in child-process lifecycle management.

**Diagnosis:**

- **PID-slot impact:** 4,544 / 4,194,304 = 0.1% — benign at current scale but demonstrates that nexus never reaps its children.
- **Self-clears only on nexus restart** — zombies are reparented to init (ppid=1) only when the parent nexus process dies.
- Historical accumulation from earlier bugs now fixed:
  - Worktree-gate spawn loop: fixed in commit `c1aeff64`
  - Crash-loop false-positive: fixed in commit `3183bf5b`

**Root cause** (from inspection of `hex-nexus/src/orchestration/agent_manager.rs::spawn_local_agent`):

- Method stores `std::process::Child` in `self.local_children: Mutex<Vec<LocalAgent>>` at line ~672.
- The mutex is held but **nobody ever calls `child.try_wait()` or `child.wait()`** on the stored handles.
- When a hex-agent child exits, nothing reaps it → the kernel keeps the entry as `Z` (zombie), holding a PID slot indefinitely.
- Per POSIX contract: the parent (nexus) must call `waitpid()` to release the zombie; until then, it remains in the process table.

**Companion modules that do NOT reap:**

1. **`hex-nexus/src/orchestration/zombie_sweeper.rs`** — logs zombies with `ppid=1` (already orphaned to init), cannot reap them (only init can).
2. **`hex-nexus/src/orchestration/orphan_reaper.rs`** (commit `0f3bb916`) — reaps *live-but-unregistered* hex-agent processes via `SIGTERM`/`SIGKILL`. When the reaper successfully kills a process, that process *becomes* a zombie that also goes unreaped.
3. **`spawn_local_agent`'s existing supervisor_subscriber watchdog** (`hex-nexus/src/orchestration/supervisor_subscriber.rs` ~line 258) — polls `/proc/<pid>` to detect exit, but does not call `waitpid()`.

All three modules observe or create zombies; none of them reap nexus-owned children.

---

## Decision

Introduce a **periodic `try_wait()` pass over `self.local_children`**, run by a new background task or extended into the existing `OrphanReaper` tick (every 60s). The implementation will:

1. **Acquire the `local_children` mutex** and iterate over all stored `LocalAgent` handles.
2. **Call `child.try_wait()`** (non-blocking) on each handle.
3. **On `Ok(Some(status))`**: the child has exited — log the PID and exit code, then drop the handle from the vec.
4. **On `Ok(None)`**: the child is still running — leave the handle in place.
5. **Emit metrics** via the `supervisor_event` audit log (same pattern as `zombie_sweeper` and `orphan_reaper`): `{ "reaped": N, "still_alive": M }`.

**Integration point:** Extend `OrphanReaper::tick()` with a second phase that walks `agent_manager.local_children` and reaps any exited handles. This is the natural home because:

- The reaper already walks `/proc` every 60s and is lifecycle-aware.
- It already has an escalation loop (`pending_kill` set) — adding a reap pass fits the same supervision domain.
- It covers **both** nexus-spawned children **and** ex-orphans the reaper killed (those also become nexus's zombies after `SIGKILL`).

**Alternative considered:** A standalone `ChildReaper` task. Rejected — adds another background loop for a single responsibility already covered by `OrphanReaper`'s 60s tick.

---

## Consequences

**Positive:**

1. **Zombie count converges to zero** on the next nexus startup + steady-state operation. No more PID-slot accumulation.
2. **Deterministic reaping** — every exited hex-agent child is reaped within 60s (one `OrphanReaper` tick).
3. **Unified supervision** — `OrphanReaper` becomes the single source of truth for live/dead worker lifecycle (SIGTERM/SIGKILL for orphans, `try_wait()` for owned children).
4. **No new task overhead** — extends an existing 60s loop rather than adding a new poller.

**Negative:**

1. **Up to 60s latency** before a zombie is reaped. Mitigated: zombies consume only a PID slot (no memory/CPU); 60s matches the existing supervisor tick cadence.
2. **`OrphanReaper` now touches `AgentManager`'s internal state** — requires shared access to `local_children: Mutex<Vec<LocalAgent>>`. Implementation must hold the mutex briefly (< 1ms per handle) to avoid blocking spawns.

**Verification:**

- After implementation: restart nexus, wait 2 minutes, run `ps aux | grep defunct | grep hex-agent | wc -l` — should be 0.
- Monitor `supervisor_event` rows for `kind="child_reap"` with payload `{ "reaped": N }` — confirms periodic reaping is active.

**Files modified:**

- `hex-nexus/src/orchestration/orphan_reaper.rs` — add reap phase to `tick()`, pull `local_children` from `SharedState.agent_manager`.
- `hex-nexus/src/orchestration/agent_manager.rs` — expose `local_children: Arc<Mutex<Vec<LocalAgent>>>` via `pub fn local_children(&self)` accessor.
- `hex-nexus/src/state.rs` — ensure `SharedState.agent_manager` is an `Arc<AgentManager>` (already true per line 672 contract).

**References:**

- Upstream spawn-loop fixes: `c1aeff64` (worktree-gate), `3183bf5b` (crash-loop FP)
- Companion module: `orphan_reaper.rs` commit `0f3bb916`
- POSIX `waitpid(2)` contract: https://man7.org/linux/man-pages/man2/waitpid.2.html

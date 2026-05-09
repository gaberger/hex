# ADR-2605082200 — Resource-Aware Supervisor (process_observation + tick)

Status: **Accepted** (implementation in flight)
Date: 2026-05-08
Supersedes: none
Related: ADR-027 (HexFlo coordination), ADR-2605081126 (worktree-mandatory merge gate), feedback_supervisor_in_stdb, feedback_cpu_pinning_diagnosis

## Context

The supervisor surface in hexflo-coordination tracks **liveness** but ignores
**resource utilisation**. Concretely:

| Observed today (2026-05-08, ~17:50) | Why supervisor missed it |
|---|---|
| Two `hex-agent daemon` PIDs, same `agent-id`, ~60 GB combined RSS | Same logical agent → supervisor saw "1 alive" |
| Swap saturated 15 / 15 GiB (9 MiB free) | No memory pressure metric anywhere |
| ollama runner pinned at 1560 % CPU (qwen2.5-coder:32b on CPU, GPU 0 %) | No CPU/GPU watcher |
| Defunct `hex-agent` zombie pid 1917696 | No `/proc` walker — zombies never observed |

The persona supervisor (`persona_pool` / `persona_health` / `persona_tick`)
only consumes inference success/failure; the worker supervisor
(`worker_pool_intent` / `worker_process` / `supervisor_tick`) watches
restart counts but is fed exclusively by processes spawned through
nexus — externally-spawned processes (operator, prior session, OOM-killed
respawns) are invisible.

This is the gap behind every "all my CPUs are pinned" question and every
operator surprise about RSS. We need to close it inside the same
STDB-resident supervisor pattern (per `feedback_supervisor_in_stdb.md`),
not via a shell loop.

## Decision

Add a **resource observer** path entirely inside the existing supervisor
architecture:

1. **STDB schema** (in `hexflo-coordination`):
   - `process_observation` — one row per live PID, upserted every 15 s.
     Columns: `pid` (pk), `host`, `argv_sha`, `argv_first`, `state`,
     `ppid`, `started_micros`, `rss_kb`, `cpu_pct`, `observed_at`.
   - `resource_anomaly` — append-only audit trail. Columns: `id`
     (auto pk), `detected_at`, `kind ∈ {duplicate_argv, rss_oversize,
     zombie, cpu_pin}`, `severity ∈ {info, warn, critical}`, `pids`
     (JSON list), `note`, `handled` (bool), `handled_at`, `handled_by`.
   - `resource_supervisor_tick_schedule` — the schedule anchor for the
     scheduled reducer.
2. **Reducers**:
   - `process_observation_upsert(pid, host, argv_sha, argv_first, state,
     ppid, started_micros, rss_kb, cpu_pct)` — upsert one PID.
   - `process_observation_prune(stale_seconds)` — drop rows whose
     `observed_at` is older than threshold (PID is gone).
   - `resource_supervisor_init()` — idempotent schedule seed.
   - `resource_supervisor_tick(_)` — scheduled every 60 s. Scans
     `process_observation`, emits `resource_anomaly` rows for:
     - duplicate `argv_sha` with > 1 alive PID (excluding system/kernel
       argvs)
     - RSS > 20 GiB → severity `warn`, > 30 GiB → `critical`
     - state = `Z` (zombie) → `critical`
     - cpu_pct > 800 % sustained for two ticks → `warn`
   - `resource_anomaly_ack(id, handled_by)` — operator/nexus marker.
3. **Nexus side**:
   - `orchestration::resource_observer` tokio task: walks `/proc` every
     15 s, computes `argv_sha`, RSS (from `/proc/<pid>/status`), CPU %
     (delta of `/proc/<pid>/stat` cumulative jiffies between ticks),
     `ppid`, `state`, `started_micros`, calls
     `process_observation_upsert` for each interesting process.
     "Interesting" filter = the argv command-name matches one of:
     `hex-nexus`, `hex-agent`, `hex`, `ollama`, `spacetimedb-stand`,
     `spacetimedb-up`, `claude`. Operators tune the allow-list via
     `HEX_RESOURCE_OBSERVER_ALLOW` env (CSV of comm prefixes).
   - On boot, nexus calls `resource_supervisor_init` (idempotent) so the
     tick fires whether the operator publishes a fresh module or not.
4. **Operator surfaces**:
   - REST: `GET /api/resources` (current observations),
     `GET /api/resources/anomalies` (open + recent), `POST
     /api/resources/anomalies/ack` (mark handled).
   - Dashboard: `#/resources` Solid view — top table sorted by
     RSS, anomaly badges, ack buttons.
5. **No auto-kill, ever.** This ADR ships **observation + alerts only**.
   Auto-reap of zombies and operator-confirmed kill of
   duplicate-argv processes is a follow-on workplan that requires its
   own ADR (kill is destructive, must be opt-in).

### Why upsert and not append-only

The naive append-only "process_sample" log would grow unbounded; pruning
keeps `process_observation` at ~ 100 rows and lets the tick reducer
do simple SELECT-and-iterate. The forensic trail lives in
`resource_anomaly` (which IS append-only and is bounded only by ack +
manual GC).

### Why a 15 s walker / 60 s tick

15 s is short enough to catch a runaway before swap thrashes; 60 s for
the anomaly tick avoids blanketing the operator inbox with transient
spikes. Both are tuneable via ScheduleAt updates without code changes.

## Consequences

Positive:
- Operator gets first-class visibility into RSS / CPU / duplicates with
  the same dashboard cadence as everything else.
- Future "auto-cleanup" workplans have a stable substrate to subscribe
  to (`resource_anomaly`).
- Closes the standing footgun where prior-session daemons survive
  invisibly.

Negative:
- `/proc` walker is Linux-only (we already are). macOS dev nodes will
  log a one-shot warning and disable the observer.
- Adds ~ 100 rows + ~ 1 KB / row to STDB persistent state. Negligible
  next to the existing tables.

## Validation

- A deliberately-spawned duplicate `hex-agent daemon` produces a
  `duplicate_argv` anomaly within ≤ 75 s.
- `kill -STOP <pid>` followed by `kill -9 <pid>` produces a `zombie`
  anomaly within ≤ 75 s and `process_observation_prune` removes the
  entry within two ticks of the PID disappearing.
- The `swap_saturated` reproduction (60 GB allocator) flips the
  responsible PID's row to `rss_oversize` severity = `critical`.

## Out of scope

- Auto-kill / OOM-killer integration (separate ADR).
- Per-cgroup quotas (would require systemd integration).
- Network / FD counts (next iteration if RSS+CPU isn't enough).

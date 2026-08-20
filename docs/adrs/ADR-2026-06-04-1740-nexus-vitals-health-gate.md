# ADR-2026-06-04-1740 — nexus-vitals-health-gate

Status: **Proposed**
Date: 2026-06-04
**Applies-To:** hex-nexus orchestration loops, hex doctor, hex nexus start, inference calibration probe, swarm productivity tracking, hex-cli/src/commands/nexus.rs
**Superseded-By:** none

## Context

On 2026-06-04 an operator asked "what the fuck is running now" and "how do we ensure
this doesn't continue" after suspecting days were wasted. Investigation found the
nexus daemon had been running for **4 days 17 hours** while producing **zero committed
output** from 3 nominally-"active" swarms (`brain-lease`, `ebay-mvp`, `wp-ebay-ui`).
`git log --since="4 days ago" -- examples/ebay-clone` returned nothing.

Autopsy surfaced **four latent defects, none of which is "forgot to build release":**

1. **Silent busy-spin — ROOT CAUSE FOUND & CONFIRMED 2026-06-04.** The nexus daemon
   burns ~6 cores (300–630% CPU) at idle with zero log growth. perf on a symbol build
   showed self-CPU dominated by `serde_json::Value::clone` / `deserialize_any` /
   `drop_in_place<Value>` + glibc malloc churn (`_int_malloc`/`_int_free`) +
   `__lll_lock_wait_private` (the **malloc arena lock**). `perf trace` showed one worker
   issuing **331,848 `futex` calls in 4s** (36% EAGAIN). Mechanism: every STDB adapter
   parses SQL responses into **untyped `serde_json::Value`** (`sql_query() -> Vec<Value>`)
   and clones rows; ~20 short-interval poll loops (org_responder 4s, supervisor/integrator
   5s, brain_progress 10s, action_executor/reconciler 15s) call these on growing
   `SELECT *` tables (`rl_q_entry`, `rl_pattern`, `agent`, `message`, `workplan_task`).
   That allocation firehose is funneled through **only 2 malloc arenas** because
   `MALLOC_ARENA_MAX=2` is set by `hex-cli/src/commands/nexus.rs:417` (ADR-2026-05-22-1720),
   so all workers thrash 2 arena locks → the futex storm. **PROVEN reversibly**: restarting
   with `MALLOC_ARENA_MAX=16` dropped idle CPU from ~600% to **0%** (RSS 1.1 GB). So
   ADR-2026-05-22-1720 traded a 25 GB memory problem for a 6-core CPU problem — and its own
   notes dismissed the "100% futex_wait" reading as "normal tokio behavior," which was wrong.

2. **Debug binary preferred over release.** `hex-cli/src/commands/nexus.rs:27`
   resolves `["debug", "release"]` in that order, so `hex nexus start` can never pick
   the release binary in a dev checkout unless `HEX_NEXUS_BIN` is set manually. A
   release binary built 2026-05-30 19:18 sat unused for 5 days while the debug daemon ran.

3. **Calibration never auto-runs.** `config_sync.rs:248` preloads inference providers on
   startup and *preserves* any existing quality_score, but registers new/uncalibrated
   providers at the default `0.0` and **never triggers calibration**. `calibrate-all`
   fires only on a manual `POST /api/inference/calibrate-all` or `hex inference test`, so
   3 of 4 providers sat at `q=0.00` indefinitely — not "scored zero" (the scorer's floor
   on a successful probe is ~0.40) but "never calibrated." CORRECTED 2026-06-04: an
   earlier read of this ADR blamed a calibration probe sending a model literally named
   `probe`; that `model=probe → HTTP 404` log line is an *unrelated* startup health-check,
   not the calibration path (`run_calibration` sends the provider's real primary model).
   Verified by running calibrate-all by hand: ollama-qwen25-32b 0.00→0.85,
   tenstorrent-qwen3-32b 0.00→0.92, deepseek-r1 0.50→0.99; the only true failure was
   `qwen3.6:35b-a3b` (OOM: needs 9.7 GiB, 8.4 free — and a reasoning-MoE unfit for codegen
   per memory, now de-registered).

4. **No watchdog was running.** `hex brain daemon-status` → "sched daemon not running",
   so the self-improvement / auto-fix loop that should have flagged any of the above
   never executed.

**Common root cause:** nothing observes nexus's *own* vitals. Every signal needed to
catch this in minutes was observable — CPU at idle, provider quality, commits-vs-"active"
— but nothing was watching. Liveness (heartbeat) was tracked; productivity and
self-health were not. Cf. the standing lesson "it compiles ≠ it works"; here, "active ≠
producing."

## Decision

Add a **nexus vitals health gate** with three enforced guardrails (operator-selected
2026-06-04):

1. **Hard health gate** — `hex doctor` gains a `vitals` check (also run at swarm-dispatch
   and surfaced by `hex go`/`hex pulse`) that HARD-FAILS when:
   - nexus self-CPU exceeds a threshold while no inference is in flight (busy-spin),
   - any registered provider is `q=0.00`,
   - STDB is unreachable, or
   - the daemon is a debug build.

2. **Productivity watchdog** — a nexus background task flags any swarm `active` for
   > N hours with zero commits and zero completed tasks, then auto-pauses + alerts via
   the inbox (priority-2). Kills the "spinning into the void" mode directly.

3. **Auto-release rebuild** — `hex nexus start` prefers `target/release` for the
   long-lived `--daemon` path (debug only via explicit `HEX_NEXUS_BIN` or a `--dev`
   flag), and warns loudly if it ever resolves a debug binary as a daemon.

Plus two concrete bug fixes that fall out of the autopsy:

4. **Fix the busy-spin (root cause now known — supersedes ADR-2026-05-22-1720's approach).**
   The arena cap is the wrong lever: raising `MALLOC_ARENA_MAX` fixes CPU but reignites the
   25 GB RSS bloat; keeping it at 2 fixes RSS but burns 6 cores. Resolve BOTH by attacking
   the allocation rate and the allocator:
   (a) **Swap the global allocator to jemalloc or mimalloc** (per-thread caches, no global
   arena-lock storm) — this fixes the original mmap/RSS fragmentation AND the lock
   contention, allowing `MALLOC_ARENA_MAX` to be removed entirely. Validate over a multi-hour
   run that RSS stays bounded AND idle CPU stays low.
   (b) **Cut the allocation firehose**: deserialize STDB rows into typed structs via serde
   derive instead of `serde_json::Value` + per-row clone; widen/cache the 4–5s poll loops or
   move them to STDB push-subscriptions instead of `SELECT *` polling.
   Add a regression test asserting idle nexus CPU stays below a ceiling (this is what the
   health gate in decision 1 enforces at runtime).

   **IMPLEMENTED 2026-06-04** (path (a) shipped): hex-nexus now uses jemalloc as its
   `#[global_allocator]` (`tikv-jemallocator` under `[target.'cfg(unix)'.dependencies]`,
   `static GLOBAL` in `src/bin/hex-nexus.rs`), and the `MALLOC_ARENA_MAX=2` default was
   removed from `hex-cli/src/commands/nexus.rs`. Validated on a clean release daemon: arena
   cap unset, **idle CPU 600% → 0%**, RSS ~1.08 GB, STDB + inference + swarms functional;
   jemalloc holds 0% even with `MALLOC_ARENA_MAX=2` still in env (it owns the Rust heap).
   This **supersedes ADR-2026-05-22-1720**. Still open (path (b), lower priority, separate
   task): cut the allocation rate via typed STDB deserialization / push-subscriptions; and a
   multi-hour RSS watch to confirm jemalloc bounds the original 25 GB bloat.

5. **Auto-calibrate registered providers.** After `config_sync` preload (and on any new
   provider registration), best-effort async-trigger `calibrate-all` for providers whose
   score is the uncalibrated default. Distinguish "uncalibrated" (sentinel, e.g. `-1.0`)
   from a real scored `0.0` so routing and the health gate can tell "never measured" from
   "measured bad." DONE 2026-06-04 (operational): ran calibrate-all by hand → 3 providers
   now 0.85/0.92/0.99; removed the OOM qwen3.6 provider. The remaining work is making this
   automatic so it survives the next restart/registration without a human.

## Consequences

- A broken substrate can no longer masquerade as healthy for days; the gate fails loud
  and blocks dispatch onto it.
- Dev iteration on a debug nexus still works via the explicit opt-in, but is no longer
  the silent default for a multi-day daemon.
- Requires the sched/brain daemon to actually be running for the watchdog to fire —
  reinforces CLAUDE.md rule 5 (start the brain daemon at session start).

## Status note

Authored directly rather than via persona-SOP because the SOP path runs on the
inference substrate (q=0.00 local providers) that is itself the subject of this ADR —
the documented "hex surface compromised → bypass" exception. Promote to **Accepted**
once the workplan below lands the gate.

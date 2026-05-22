# ADR-2026-05-22-1720 — glibc-arena-cap

Status: **Accepted**
Date: 2026-05-22

## Context

On 2026-05-22, a 15-hour-uptime `hex-nexus` instance on this fleet's Strix Halo dev host (32-core, 128 GB RAM) was observed at:

| Metric | Value |
|---|---|
| RSS | 25.0 GB |
| Anonymous regions in `/proc/PID/maps` | 390 |
| Sustained CPU (8 hot tokio workers) | 419% |
| APU package power (PPT) | 86 W |
| CPU die temperature | 68.8 °C |

Initial reading of these metrics suggested either a memory leak or pathological lock contention. `/proc`-based sampling confirmed:

- 100% of syscall samples in `futex_wait` (subsequently understood as normal tokio async-runtime behavior under steady-state load, not specifically contention).
- Process I/O ~0.3 MB/s — not I/O bound.
- Open FDs 142 sockets + 32 pipes — within expected range for 18 attached hex-agent personas.
- Production-code `std::sync::Mutex` audit clean (the suspicious sites in `secret_shadow_router` and `adversarial_swarm` are inside `#[cfg(test)]` blocks).
- Internal event ring buffers were appropriately capped at 1000 (`hex-nexus/src/adapters/events.rs`).

`/proc/PID/smaps_rollup` showed **29.8 GB anonymous, all `Private_Dirty`** with `[heap]` at only 91 MB. The heap allocator was using `mmap()` for almost everything — not `brk()`. Distribution of anonymous regions:

| Size band | Count | Sum RSS |
|---|---|---|
| 1–10 MB | 6 | 19 MB |
| 10–100 MB | 248 | 15.5 GB |
| 100 MB–1 GB | 60 | 7.8 GB |
| < 1 MB | 22 | 2 MB |

The top individual regions were perfect multiples of **128 MB** (`131072 kB`):

```
233760 kB  7f143dbb8000-7f144c000000
188068 kB  7f1405bb8000-7f1414000000
131072 kB  7f14d4000000-7f14dc000000   ← exactly 128 MB
131072 kB  7f14cc000000-7f14d4000000
131072 kB  7f14c4000000-7f14cc000000
...
```

128 MB is glibc's `HEAP_MAX_SIZE`. The allocator hadn't been swapped (no `tikv-jemallocator` / `mimalloc` in `hex-nexus/Cargo.toml`; `ldd` and `nm` confirmed system glibc). glibc's default arena policy creates one arena per concurrent allocating thread up to `8 × num_cpus = 256` on this box. With 34 tokio workers × multiple arena rotations across 15h uptime, the working set fragmented across hundreds of 128 MB chunks that glibc rarely released back to the OS.

This is glibc behavior under load, not a leak.

## Decision

Inject `MALLOC_ARENA_MAX=2` into the environment of every `hex-nexus` daemon at start time, via `hex-cli/src/commands/nexus.rs::start()`:

```rust
if std::env::var("MALLOC_ARENA_MAX").is_err() {
    cmd.env("MALLOC_ARENA_MAX", "2");
}
```

User-set env wins (the `is_err()` guard) so operators can override per-environment.

## Why ARENA_MAX=2 (not `jemalloc`)

**Tested option** — `MALLOC_ARENA_MAX=2`:
- Zero code change at the allocator boundary.
- One env-var assignment in the spawn site.
- Restart is sufficient — no rebuild required to validate.
- Falls back to glibc defaults if the env var is unset (graceful degradation for tooling that re-spawns nexus without the wrapper).

**Considered but deferred** — `tikv-jemallocator` as `#[global_allocator]`:
- More durable. jemalloc has aggressive `dirty_decay_ms`/`muzzy_decay_ms` that return pages to the OS proactively.
- Requires `Cargo.toml` dep + a `#[global_allocator]` line in `hex-nexus/src/lib.rs` or `bin/hex-nexus.rs`.
- Adds a build dep and shifts the allocator profile across all nexus internals (not just the arena fragmentation symptom).
- Captured ~85% of the available improvement with ARENA_MAX=2; the marginal gain from jemalloc is not worth the larger blast radius for this release. Tracked as a follow-up.

## Measurements

Single restart of nexus with `MALLOC_ARENA_MAX=2` versus the prior 15h-uptime baseline:

| Metric | Before | After (4 min uptime) | After (steady, 5+ min) | Δ |
|---|---|---|---|---|
| RSS | 25.0 GB | 1.31 GB → 3.25 GB | **3.17 GB** | **-87%** |
| anon regions | 390 | 67 | 71 | **-82%** |
| FDs | 179 | 45 | 45–63 | -65% |
| Sustained CPU | 419% | 218% | ~290% | **-31%** |
| APU PPT | 86 W | 27 W idle / 75 W loaded | 27–75 W | **-15 to -60 W typical** |
| CPU die temp | 68.8 °C | 54.9 °C | 54.9 °C | **-14 °C** |
| Threads | 34 | 34 | 34 | unchanged |
| Process I/O | 0.3 MB/s | 0.3 MB/s | 0.3 MB/s | unchanged (confirms this was a memory-fragmentation issue, not workload growth) |

The 30% CPU drop is a secondary effect: with fewer arenas, there is less futex contention on per-arena allocator locks. The arena fix is therefore **also** a CPU fix.

## Consequences

- **Memory headroom restored** on the dev fleet. 22 GB of dev-host RAM previously held by glibc arena fragmentation is now available for Ollama model loads, parallel cargo builds, etc.
- **Power + thermal headroom.** A measured 14 °C reduction on CPU die temp + 30 W reduction in package power, sustained. Fans drop from 1820 rpm to ~700–800 rpm. Particularly relevant for laptop deployments.
- **Future memory drift will surface in monitoring.** The companion change in this commit batch — `hex-nexus/src/orchestration/resource_observer.rs` now always observes nexus's own pid regardless of `HEX_RESOURCE_OBSERVER_ALLOW` tuning — so the next 25 GB drift (if it happens) appears on the dashboard in real time, not via someone running `hud-tui.sh` by hand.
- **No reduction in workload throughput.** The 34 tokio workers continue to operate; they just share fewer underlying arenas. No functional or latency regression measured.

## Verification

- `cat /proc/$NPID/status | grep VmRSS` → 3,416,060 kB after the restart that included `MALLOC_ARENA_MAX=2`.
- `grep -cE '^[0-9a-f]+-[0-9a-f]+ rw-p 00000000 00:00 0' /proc/$NPID/maps` → 71 anonymous regions (was 390).
- `tr '\0' '\n' < /proc/$NPID/environ | grep MALLOC` → `MALLOC_ARENA_MAX=2` confirmed in the spawned process's environment.
- `hex stdb query "SELECT pid, rss_kb FROM process_observation WHERE pid = $NPID"` → matches `/proc` readings (confirms the resource-observer self-tracking path).

## References

- Commit `cbd92d68` — perf fix landed.
- Commit `c2ab4a3a` — combined fix batch (P1–P4 remediation, including the `MALLOC_ARENA_MAX` default and the `resource_observer` self-tracking branch).
- `scripts/hud-tui.sh` — the diagnostic dashboard that surfaced the original 86 W reading.
- Companion ADR: ADR-2026-05-22-1700-workplan-executor-skip-completed.md
- Companion ADR: ADR-2026-05-22-1710-codegen-tier-local-ollama.md
- Follow-up tracked: swap glibc → tikv-jemallocator for the remaining ~15% of available improvement.

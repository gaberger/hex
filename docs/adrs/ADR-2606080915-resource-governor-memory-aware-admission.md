# ADR-2606080915: Resource governor — memory-aware admission control + model residency

**Status:** Accepted
**Date:** 2026-06-08
**Epoch:** hybrid-inference
**Drivers:** A measured OOM. Loading qwen3-coder-next Q2_K (29 GB) to benchmark it, *while* the
benchmark ran compile-heavy fixtures (cargo/tsc/go), exceeded the box's 30 GB RAM — the kernel OOM
killer took down the run *and* unrelated background processes. The audit that followed found hex has
no memory-aware scheduling at all: it is an "AI Operating System" missing the most basic OS function,
a memory manager.
**Supersedes:**
**Superseded-By:**

## Context

CPU/disk spillover is **not** the gap — ollama/llama.cpp already provide it (qwen-next ran 15 GB on
GPU + 29 GB mmap'd from disk and answered fine). The gap is **governance**: nothing in hex reasons
about the RAM/VRAM budget before it acts. An audit of `hex-nexus`, `hex-exec`, `hex-core` found:

1. **No memory-aware admission control.** Nothing checks `MemAvailable`/VRAM-free before loading a
   model or dispatching a memory-heavy job. A 29 GB model-load and a compile-heavy benchmark were
   allowed to collide on 30 GB RAM.
2. **Model residency delegated to ollama's blind LRU.** hex doesn't own which models are resident.
   It has already been *bitten* by this and band-aided it with a one-off mutex —
   `sched_service.rs`: *"qwen2.5-coder:32b ≈ 19.8 GB vs nemotron-mini ≈ 2.7 GB… together exceed
   typical local GPU VRAM, so Ollama evicts whichever model is least-recently-used… the autopilot's
   qwen load during a tick can evict nemotron mid-RL-cycle and the RL inference times out (observed
   2026-04-27)."* That mutex is a point-fix for one pair of callers; the general problem is unowned.
3. **No memory-pressure degradation.** hex has a `Degraded` health state, but only for *provider
   unreachable* — not for *memory tight → route to a smaller model / shrink context / fall back to
   `claude -p`*.
4. **No sandbox memory isolation.** Benchmark/loop compilation (cargo/tsc/go) runs unbounded; it can
   starve inference (and vice versa). No cgroup/ulimit.
5. **Off-box inference is scaffolded but unused.** The multi-host substrate (ADR-040,
   ADR-2026-05-09-1100) has ports + transport, but heavy inference is co-located with the
   compile-heavy loop instead of routed to a dedicated inference host.

The pointed framing: a real OS won't let a process allocate past physical memory and crash unrelated
processes. hex did exactly that. For an AIOS, a resource governor is table stakes, not a feature.

## Decision

Introduce a **resource governor** in hex: a memory-aware admission/scheduling layer consulted before
any heavy operation (model load, sandbox/compile dispatch). Define an **`IResourceGovernor` port** in
`hex-core` (query the memory budget; admit / queue / deny / route a workload) with a secondary adapter
in `hex-nexus` that reads real system state (`sysinfo` / `/proc/meminfo` + `nvidia-smi`). The
composition root wires it; the inference gateway and the bench runner consult it.

Delivered in tiers, smallest-first:

- **Tier 0 — admission control (the OOM fix).** Before loading a model or dispatching a sandbox job,
  estimate its resident footprint (quant size + KV-cache headroom; job working-set headroom) against
  `MemAvailable`/VRAM-free. If it won't fit with headroom: **refuse, queue, serialize, or route to
  the `claude -p` fallback** (which needs no local RAM). Never co-schedule a model-load and a
  compile-heavy job that together exceed RAM. Log *why* every deferral/route happened.
- **Tier 1 — residency management.** hex owns model residency: evict-before-load under pressure, set
  `keep_alive` intentionally per workload, serialize big-model loads. Generalizes the `sched_service`
  mutex into one owner instead of per-caller locks.
- **Tier 2 — pressure degradation.** Extend `Degraded` to memory: under pressure, route to a smaller
  model / shrink context / defer compilation / fall back to frontier — graceful, not OOM.
- **Tier 3 — sandbox isolation.** Bound sandbox compilation with cgroup/ulimit memory caps so the
  loop and inference can't starve each other.
- **Tier 4 — host separation (the durable answer).** Wire the existing multi-host substrate so a
  model exceeding the local budget runs on a dedicated inference host/process, off the box doing the
  compile-heavy loop. The transport exists; the governor becomes its routing trigger.

## Consequences

**Positive**
- Eliminates the OOM class — hex stops taking itself and unrelated processes down.
- hex earns the "OS" claim in the memory dimension (admission + residency + degradation).
- On-thesis with best-of-N (ADR-2606072044): the local-vs-`claude -p` choice becomes a *measured
  resource-fit* decision, not a guess. A model that won't fit is routed to frontier automatically.
- Makes "big model on a small box" *safe* — it routes off-box or to frontier instead of OOMing,
  turning tonight's hard wall into a graceful fallback.

**Negative / risks**
- Footprint estimation is approximate (KV cache scales with context) — err conservative, leave
  headroom; tune from observed loads.
- Admission control adds pre-flight latency and can *refuse* work; it must be transparent (log the
  reason, surface on the dashboard), never a silent drop.
- Tier 4 is a substantial build; Tier 0 is the high-ROI start and stands alone.

## Implementation

- `hex-core`: `IResourceGovernor` port — memory-budget query + admit/queue/deny/route decision type.
- `hex-nexus` secondary adapter: system memory/VRAM reader (`sysinfo` or `/proc/meminfo` +
  `nvidia-smi`), behind the port.
- Consumers: the inference gateway and the bench runner consult the governor before model-load /
  sandbox dispatch; on insufficient budget they queue or route to `claude -p`.
- Sequence: **Tier 0 admission first** (closes the OOM), then residency → degradation → isolation →
  host-separation as the backlog this ADR opens.
- Tracking workplan: create via `hex plan draft`.

## References

- The OOM incident (2026-06-08) + memory `project_qwen_next_hardware_ceiling` — the measured driver.
- `hex-nexus/src/sched_service.rs` — the one-off model-eviction mutex this generalizes.
- ADR-2606072044 — best-of-N + `claude -p` fallback (the governor's route-to-frontier target).
- ADR-040, ADR-2026-05-09-1100 — multi-host substrate (Tier 4 host separation).
- ADR-2606071340 — nexus hexagonal compliance / crate split (the port-and-adapter pattern this follows).

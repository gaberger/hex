# wp-idle-research-swarm — Idle Research Swarm

**ADR:** [ADR-2604151200 — Idle Research Swarm](../adrs/ADR-2604151200-idle-research-swarm.md)
**Status:** Stub
**Companion JSON:** [`wp-idle-research-swarm.json`](./wp-idle-research-swarm.json)

## Objective

Use sched-daemon idle time to run a tiered research swarm over the codebase that
emits **structured findings** (not freeform Markdown) and routes each finding
back into the existing draft-ADR / draft-workplan / memory surfaces so insights
become actionable inputs instead of dead reports.

Operationally: when `queue_drain()` sees zero pending and zero in-flight tasks
for `idle_threshold_ticks` consecutive ticks, the daemon self-enqueues a
`research-sweep` task — throttled to once per `min_sweep_interval_h` (default
6 h) — that runs deterministic analysts first (T1) and only escalates to
T2 / T2.5 LLM synthesis to convert verified evidence into concrete suggestions.

## Tasks (placeholder)

Detailed task graph lives in `wp-idle-research-swarm.json`. High-level phases:

- **P1 — Idle-trigger + schema**
  - Idle-tick tracking and self-enqueue of `kind: research-sweep` in `hex-cli/src/commands/sched.rs`.
  - Throttle via `~/.hex/sched/last_research_sweep`.
  - Structured `Finding` type in `hex-core/src/research_finding.rs` with serde_yaml round-trip.
- **P2 — Deterministic (code-first) analysts (T1)**
  - Architecture analyst over `hex analyze .`.
  - Code-quality analyst over `cargo check --workspace` + clippy.
  - Size/complexity over `tokei` / `scc`; activity heatmap from `git log --since=30d`.
  - Workplan/ADR drift via `hex plan reconcile --dry-run`.
- **P3 — LLM synthesis analysts (T2 / T2.5)**
  - Naming-convention walk (T2, tree-sitter).
  - Performance / scaling / UI-UX analysts (T2.5) — synthesize deterministic evidence into suggestions; never invent findings without evidence.
- **P4 — Routing back into the system**
  - `kind: adr` → draft under `docs/adrs/drafts/` (no auto-promote).
  - `kind: workplan` → draft under `docs/workplans/drafts/`.
  - `kind: memory` → `hex memory store` namespace `idle-sweep`.
  - Hard cap `max_drafts_per_sweep: 5`.
- **P5 — Visibility**
  - Sched status line: `last_sweep: <age> (N findings, M promoted)`.
  - Dashboard panel surfacing sweep history + promoted drafts.
- **P6 — Preemption**
  - Sweep aborts cleanly and re-queues itself when a non-research task lands during execution.

> Maintain canonical task IDs in `wp-idle-research-swarm.json`; this file is the
> human-readable narrative that mirrors that JSON.

## Dependencies

- **ADR-2604151200** (this feature's decision record).
- **ADR-2604142345** — insight routing (findings must route to drafts, not dead `.md`).
- **ADR-2604141400** — sched queue swarm-lease (single-writer guarantee for self-enqueue).
- **ADR-2604131630** — code-first execution (T1 deterministic gating before T2/T2.5 inference).
- **ADR-2604110227** — task-tier routing (T1/T2/T2.5 dispatch).
- Existing CLI surfaces: `hex analyze`, `hex plan reconcile`, `hex plan drafts`, `hex memory store`.
- Local Ollama tier models per `.hex/project.json` → `inference.tier_models`.
- `serde_yaml` for the structured finding schema.

## Success Criteria

1. **Idle-trigger correctness.** Daemon enqueues `research-sweep` after exactly
   `idle_threshold_ticks` consecutive empty ticks, and only when
   `now - last_research_sweep ≥ min_sweep_interval_h`. Covered by unit tests
   in `hex-cli/src/commands/sched.rs`.
2. **Throttle.** No more than one sweep per `min_sweep_interval_h` window;
   throttle persists across daemon restarts via `~/.hex/sched/last_research_sweep`.
3. **Structured output.** Every sweep writes
   `docs/analysis/idle-sweep-YYYYMMDD-HHMM.yaml` matching the `Finding` schema
   (`id`, `domain`, `severity`, `title`, `evidence[]`, `suggested_action`).
   The accompanying `.md` is a rendering, never the source of truth.
4. **Evidence-bound findings.** Every finding includes at least one concrete
   evidence entry (file:line or command output). Schema validation rejects
   findings without evidence.
5. **Routing.** Each finding with `severity ≥ med` lands as a draft ADR, draft
   workplan, or `idle-sweep`-namespace memory entry — bounded by
   `max_drafts_per_sweep: 5`. Nothing auto-promotes to Accepted; nothing
   auto-executes.
6. **Preemption.** A sweep that overlaps with an incoming non-research task
   aborts cleanly and re-queues for the next idle window — verified by an
   integration test that injects a workplan task mid-sweep.
7. **Visibility.** `hex sched status` and the dashboard report
   `last_sweep`, finding count, and promoted-draft count.
8. **Token discipline.** A sweep with zero T2.5 findings makes zero LLM calls
   (deterministic analysts only); regression-tested via a fixture repo with
   known-clean architecture.
9. **Insight-routing rule honored.** No new freeform `docs/analysis/*.md`
   files written without a matching `.yaml` source — enforced by `hex ci`.

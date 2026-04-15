# ADR-2604151200 — Idle Research Swarm

**Status:** Proposed
**Date:** 2026-04-15
**Supersedes:** —
**Related:** ADR-2604142345 (insight routing), ADR-2604141400 (sched queue swarm-lease), ADR-2604131630 (code-first execution)

## Context

The sched daemon (`hex-cli/src/commands/sched.rs`) drains its queue on a fixed tick. When the queue is empty and no tasks are in-flight, the daemon sits idle — burning a tick interval doing nothing. An AIOS that "gets smarter every run" should use that idle time.

The user explicitly asked for this: *"When things are idle we should dispatch a research swarm to review the codebase and deliver a report on improvements across the board, code structure, naming conventions, UI/UX, scaling, performance, etc."*

The naive implementation — write a long Markdown report under `docs/analysis/` — fails the **insight-routing rule** (ADR-2604142345 / `feedback_insights_are_inputs.md`): any finding that doesn't route back into an ADR draft, workplan stub, or memory entry is dead text. We have several existing reports under `docs/analysis/` that prove this failure mode.

## Decision

Add an **idle-research swarm** to the sched daemon, gated by:

1. **Idle-trigger.** When `queue_drain()` finds zero pending and zero in-flight tasks for `idle_threshold_ticks` consecutive ticks (default: 4 ticks ≈ 2 min at 30 s interval), self-enqueue a `kind: research-sweep` task.

2. **Throttle.** A research sweep only runs once per `min_sweep_interval_h` (default: 6 hours). Persisted under `~/.hex/sched/last_research_sweep`.

3. **Code-first first.** Each analyst domain runs deterministic checks before spending tokens:
   - `hex analyze .` for boundary/dead-code violations
   - `cargo check --workspace` + clippy for compile/lint health
   - `tokei` / `scc` for size + complexity
   - `git log --since=30d` for activity heatmap
   - Workplan/ADR drift via existing `hex plan reconcile --dry-run`

   Findings from these tools are zero-cost. Inference (T2.5) only runs to **synthesize** the deterministic findings into actionable suggestions.

4. **Tier the analysts.**
   | Domain | Tier | Source signals |
   |---|---|---|
   | Architecture (boundary, deadcode, cycles) | T1 | `hex analyze` |
   | Code quality (compile, clippy, complexity) | T1 | cargo / clippy |
   | Naming conventions | T2 | tree-sitter walk |
   | Performance | T2.5 | hot-path heuristics + LLM synthesis |
   | Scaling concerns | T2.5 | port/adapter coupling + LLM synthesis |
   | UI/UX (dashboard) | T2.5 | dashboard route walk + LLM synthesis |

5. **Output schema, not freeform Markdown.** Each finding is a structured record:
   ```yaml
   findings:
     - id: F-2604151200-001
       domain: architecture
       severity: high|med|low
       title: "<one-line>"
       evidence: ["<file:line>", "<command output>"]
       suggested_action:
         kind: adr | workplan | memory | noop
         draft_ref: "docs/adrs/drafts/ADR-XXXX.md"  # auto-generated stub
   ```
   The report is `docs/analysis/idle-sweep-YYYYMMDD-HHMM.yaml` (machine-readable). A rendered `.md` summary is generated alongside for humans, but the `.yaml` is the source of truth.

6. **Routing back into the system.** For each finding with `severity ≥ med`:
   - `kind: adr` → write a draft under `docs/adrs/drafts/` (NOT auto-promote to Accepted)
   - `kind: workplan` → write a draft under `docs/workplans/drafts/` (consistent with existing T3 draft flow)
   - `kind: memory` → call `hex memory store` with namespace `idle-sweep`
   The next user session sees the drafts via the existing `hex plan drafts list` and ADR review surfaces. Nothing auto-merges. Nothing auto-executes.

7. **Visibility.** The sched daemon status line includes `last_sweep: 4h ago (12 findings, 3 promoted to drafts)`. The dashboard surfaces sweeps as a first-class panel.

## Consequences

**Positive.**
- Idle compute → continuous self-review.
- Findings land as drafts in the existing review pipeline instead of dead `.md` files.
- Code-first gating keeps the per-sweep token cost bounded (most findings are deterministic; LLM only synthesizes).
- Throttle prevents runaway sweeps eating local-model bandwidth.

**Negative / risks.**
- A buggy analyst could spam draft ADRs/workplans. Mitigation: the `min_sweep_interval_h` throttle plus a hard cap of `max_drafts_per_sweep: 5`.
- Local Ollama bandwidth contention if a sweep starts just as a real workplan arrives. Mitigation: sweeps are preemptible — if the queue gains a non-research task, the sweep aborts cleanly and re-queues itself for the next idle window.
- Risk of the report being noise. Mitigation: the schema requires `evidence`, and each finding's `suggested_action` must be concrete (no "consider refactoring this" vagueness).

## Non-goals

- **Not a continuous background scan.** Sweeps run on idle, not on a fixed schedule independent of activity.
- **Not auto-promoting drafts.** Every draft passes through the existing human-in-the-loop ADR / workplan approval flow.
- **Not replacing `hex analyze`.** The architecture analyst *uses* `hex analyze`; it does not duplicate it.
- **Not running on remote agents.** Initial scope is local sched daemon only. Remote-host sweeps are a follow-up.

## Implementation

See `wp-idle-research-swarm.json`.

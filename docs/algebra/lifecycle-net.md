# Lifecycle Petri Net — 7-Phase Feature Pipeline

**Phase:** P3 of [ADR-2604111229](../adrs/ADR-2604111229-algebraic-formalization-of-process-flow.md)
**Source of truth:** `hex-cli/src/pipeline/supervisor.rs`
**Last verified against source:** 2026-04-12

---

## Overview

hex's feature development pipeline is a 7-phase workflow with fork/join
parallelism inside the Code phase. This document encodes it as a
**1-safe workflow Petri net** — a directed bipartite graph where places
hold at most one token and transitions fire when all input places are
marked.

A 1-safe workflow net has exactly one source place (input) and one sink
place (output). The net is **sound** if:

1. Every marking reachable from the initial marking can reach the final marking (completion guarantee)
2. When a token arrives in the sink place, no other place holds a token (proper termination)
3. Every transition is reachable from the initial marking (no dead transitions)

---

## The Net

### Places (circles)

Each place represents a state in the pipeline. A token in a place means
"this phase is ready to execute" or "this phase has completed."

```
P_start      — Initial state: feature request received
P_specs      — Behavioral specs written
P_plan       — Workplan decomposed into adapter-bounded steps
P_worktrees  — Git worktrees created (one per adapter boundary)
P_t0_ready   — Tier 0 (domain + ports) ready to code
P_t0_done    — Tier 0 coding complete
P_t1_ready   — Tier 1 (secondary adapters) ready to code
P_t1_done    — Tier 1 coding complete
P_t2_ready   — Tier 2 (primary adapters) ready to code
P_t2_done    — Tier 2 coding complete
P_t3_ready   — Tier 3 (use cases) ready to code
P_t3_done    — Tier 3 coding complete
P_code_done  — All tiers complete (join)
P_validated  — Validation judge returned PASS
P_integrated — Worktrees merged in dependency order
P_end        — Feature complete (cleanup done)
```

### Transitions (rectangles)

Each transition represents an action that consumes tokens from input
places and produces tokens in output places.

```
t_spec       — Write behavioral specs          (P_start → P_specs)
t_plan       — Decompose into workplan steps    (P_specs → P_plan)
t_worktree   — Create git worktrees            (P_plan → P_worktrees)
t_fork       — Fork into tier-0                (P_worktrees → P_t0_ready)
t_code_t0    — Code tier 0 (domain + ports)    (P_t0_ready → P_t0_done)
t_gate_01    — Tier barrier: 0 → 1             (P_t0_done → P_t1_ready)
t_code_t1    — Code tier 1 (secondary)         (P_t1_ready → P_t1_done)
t_gate_12    — Tier barrier: 1 → 2             (P_t1_done → P_t2_ready)
t_code_t2    — Code tier 2 (primary)           (P_t2_ready → P_t2_done)
t_gate_23    — Tier barrier: 2 → 3             (P_t2_done → P_t3_ready)
t_code_t3    — Code tier 3 (use cases)         (P_t3_ready → P_t3_done)
t_join       — Join all tiers                  (P_t3_done → P_code_done)
t_validate   — Run validation judge (BLOCKING) (P_code_done → P_validated)
t_integrate  — Merge worktrees                 (P_validated → P_integrated)
t_finalize   — Cleanup + report                (P_integrated → P_end)
```

### Net Diagram

```
                         Sequential Phases                    Code Phase (Tiered)                          Sequential Phases
                    ┌─────────────────────────┐    ┌─────────────────────────────────────┐    ┌──────────────────────────────────┐
                    │                         │    │                                     │    │                                  │

  ○ P_start        │                         │    │                                     │    │                                  │
  │                │                         │    │                                     │    │                                  │
  ■ t_spec         │                         │    │                                     │    │                                  │
  │                │                         │    │                                     │    │                                  │
  ○ P_specs        │                         │    │                                     │    │                                  │
  │                │                         │    │                                     │    │                                  │
  ■ t_plan         │                         │    │                                     │    │                                  │
  │                │                         │    │                                     │    │                                  │
  ○ P_plan         │                         │    │                                     │    │                                  │
  │                │                         │    │                                     │    │                                  │
  ■ t_worktree     │                         │    │                                     │    │                                  │
  │                │                         │    │                                     │    │                                  │
  ○ P_worktrees    │                         │    │                                     │    │                                  │
  │                │                         │    │                                     │    │                                  │
  ■ t_fork ────────┼─────────────────────────┼────┘                                     │    │                                  │
  │                │                         │                                          │    │                                  │
  ○ P_t0_ready     │                         │                                          │    │                                  │
  │                │                         │                                          │    │                                  │
  ■ t_code_t0      │                         │    Tier 0: domain + ports                │    │                                  │
  │                │                         │    (agents work in parallel on            │    │                                  │
  ○ P_t0_done      │                         │     separate worktrees within tier)       │    │                                  │
  │                │                         │                                          │    │                                  │
  ■ t_gate_01      │   BLOCKING gate         │                                          │    │                                  │
  │                │                         │                                          │    │                                  │
  ○ P_t1_ready     │                         │                                          │    │                                  │
  │                │                         │                                          │    │                                  │
  ■ t_code_t1      │                         │    Tier 1: secondary adapters             │    │                                  │
  │                │                         │                                          │    │                                  │
  ○ P_t1_done      │                         │                                          │    │                                  │
  │                │                         │                                          │    │                                  │
  ■ t_gate_12      │   BLOCKING gate         │                                          │    │                                  │
  │                │                         │                                          │    │                                  │
  ○ P_t2_ready     │                         │                                          │    │                                  │
  │                │                         │                                          │    │                                  │
  ■ t_code_t2      │                         │    Tier 2: primary adapters               │    │                                  │
  │                │                         │                                          │    │                                  │
  ○ P_t2_done      │                         │                                          │    │                                  │
  │                │                         │                                          │    │                                  │
  ■ t_gate_23      │   BLOCKING gate         │                                          │    │                                  │
  │                │                         │                                          │    │                                  │
  ○ P_t3_ready     │                         │                                          │    │                                  │
  │                │                         │                                          │    │                                  │
  ■ t_code_t3      │                         │    Tier 3: use cases                      │    │                                  │
  │                │                         │                                          │    │                                  │
  ○ P_t3_done      │                         │                                          │    │                                  │
  │                │                         │                                          │    │                                  │
  ■ t_join ────────┼─────────────────────────┼──────────────────────────────────────────┼────┘                                  │
  │                │                         │                                          │                                       │
  ○ P_code_done    │                         │                                          │                                       │
  │                │                         │                                          │                                       │
  ■ t_validate     │   BLOCKING gate         │                                          │                                       │
  │                │   (PASS required)       │                                          │                                       │
  ○ P_validated    │                         │                                          │                                       │
  │                │                         │                                          │                                       │
  ■ t_integrate    │                         │                                          │                                       │
  │                │                         │                                          │                                       │
  ○ P_integrated   │                         │                                          │                                       │
  │                │                         │                                          │                                       │
  ■ t_finalize     │                         │                                          │                                       │
  │                │                         │                                          │                                       │
  ○ P_end          │                         │                                          │                                       │
                    └─────────────────────────┘    └──────────────────────────────────────┘    └──────────────────────────────────┘
```

### Compact Notation

```
P_start → t_spec → P_specs → t_plan → P_plan → t_worktree → P_worktrees
    → t_fork → P_t0_ready → t_code_t0 → P_t0_done
    → t_gate_01 → P_t1_ready → t_code_t1 → P_t1_done
    → t_gate_12 → P_t2_ready → t_code_t2 → P_t2_done
    → t_gate_23 → P_t3_ready → t_code_t3 → P_t3_done
    → t_join → P_code_done
    → t_validate → P_validated → t_integrate → P_integrated
    → t_finalize → P_end
```

---

## Formal Definition

```
N = (P, T, F, i, o)

P = { P_start, P_specs, P_plan, P_worktrees,
      P_t0_ready, P_t0_done, P_t1_ready, P_t1_done,
      P_t2_ready, P_t2_done, P_t3_ready, P_t3_done,
      P_code_done, P_validated, P_integrated, P_end }

T = { t_spec, t_plan, t_worktree, t_fork,
      t_code_t0, t_gate_01, t_code_t1, t_gate_12,
      t_code_t2, t_gate_23, t_code_t3, t_join,
      t_validate, t_integrate, t_finalize }

F = { (P_start, t_spec), (t_spec, P_specs),
      (P_specs, t_plan), (t_plan, P_plan),
      (P_plan, t_worktree), (t_worktree, P_worktrees),
      (P_worktrees, t_fork), (t_fork, P_t0_ready),
      (P_t0_ready, t_code_t0), (t_code_t0, P_t0_done),
      (P_t0_done, t_gate_01), (t_gate_01, P_t1_ready),
      (P_t1_ready, t_code_t1), (t_code_t1, P_t1_done),
      (P_t1_done, t_gate_12), (t_gate_12, P_t2_ready),
      (P_t2_ready, t_code_t2), (t_code_t2, P_t2_done),
      (P_t2_done, t_gate_23), (t_gate_23, P_t3_ready),
      (P_t3_ready, t_code_t3), (t_code_t3, P_t3_done),
      (P_t3_done, t_join), (t_join, P_code_done),
      (P_code_done, t_validate), (t_validate, P_validated),
      (P_validated, t_integrate), (t_integrate, P_integrated),
      (P_integrated, t_finalize), (t_finalize, P_end) }

i = P_start   (source place)
o = P_end     (sink place)

Initial marking: M_0 = { P_start }
```

---

## Soundness Proof (by enumeration)

The net is a **free-choice workflow net** (every transition has exactly one
input place and one output place — it's a state machine). For state machines,
soundness reduces to:

1. **Reachability of o from i:** By inspection, the unique token flows
   `P_start → P_specs → P_plan → P_worktrees → P_t0_ready → P_t0_done →
   P_t1_ready → P_t1_done → P_t2_ready → P_t2_done → P_t3_ready →
   P_t3_done → P_code_done → P_validated → P_integrated → P_end`.
   The path is unique and covers all places. Therefore the sink is reachable.

2. **Proper termination:** When the token is in `P_end`, no transition is
   enabled (no place other than `P_end` holds a token). The net properly
   terminates because the path is linear — each transition consumes its
   input place's token before producing the output.

3. **No dead transitions:** Every transition appears on the unique
   `P_start → P_end` path. All 15 transitions fire exactly once.

**Theorem:** The hex lifecycle net is a sound 1-safe workflow net.

**Proof method:** Enumeration (the net is a sequential state machine with
16 places and 15 transitions — the reachability graph has 16 states, each
with exactly one outgoing edge). For nets this simple, enumeration is
exhaustive and sufficient. No model checker is needed.

---

## BLOCKING Gates

Three transitions in the net act as **BLOCKING gates** — they check a
predicate and only fire if it holds:

| Gate | Transition | Predicate | Source |
|:---|:---|:---|:---|
| Tier 0→1 barrier | `t_gate_01` | All tier-0 objectives pass (compile + lint + test) | `supervisor.rs` line ~1227: `run_tier(0, ...)` completes before `run_tier(1, ...)` starts |
| Tier 1→2 barrier | `t_gate_12` | All tier-1 objectives pass | Same pattern — sequential tier dispatch |
| Validation gate | `t_validate` | Validation judge returns `PASS` | `supervisor.rs` line ~1940: `gate.blocking` halts pipeline on `FAIL` |

If a BLOCKING gate's predicate fails, the transition does not fire and
the pipeline **halts**. In Petri net terms, the token stays in the input
place forever — the net does not reach `P_end`. This is correct behavior:
a failed validation should NOT produce a completed feature.

### Soundness Under Failure

The soundness proof above assumes all gates pass. Under failure, the net
is **not sound** in the classical sense (the token cannot reach `P_end`).
This is intentional — the net models a pipeline that can fail, and failure
means the token is "stuck" at the failing gate. The options are:

1. **Abort:** Human intervention removes the token (cancels the feature)
2. **Retry:** The failing transition's input is re-evaluated after a fix
3. **Escalate:** The supervisor sends an inbox notification (ADR-060)

The supervisor implements option 2 (retry loop with `MAX_ITERATIONS`)
and falls back to option 3 (inbox notification) on exhaustion.

---

## Tier Barriers as Synchronization

Within the Code phase, agents work **in parallel within a tier** but
**sequentially across tiers**. The tier barriers (`t_gate_01`, `t_gate_12`,
`t_gate_23`) are synchronization points.

In a more detailed model (not needed for soundness but useful for
understanding), each `t_code_tN` transition would expand into a
sub-net with fork/join parallelism:

```
                    ┌──→ ○ step_1 → ■ code_1 → ○ done_1 ──┐
                    │                                       │
P_tN_ready → ■ fork├──→ ○ step_2 → ■ code_2 → ○ done_2 ──├→ ■ join → P_tN_done
                    │                                       │
                    └──→ ○ step_k → ■ code_k → ○ done_k ──┘
```

The join transition requires ALL `done_*` places to have tokens before
firing. This models the supervisor waiting for all agents in a tier to
complete before advancing to the next tier.

The number of steps `k` varies per workplan and per tier. The sub-net
structure is determined at runtime by `supervisor.rs::run_tier()` based
on workplan step count.

---

## Mapping to Code

| Net element | Code location | Mechanism |
|:---|:---|:---|
| `P_start` | `hex plan execute <workplan>` invocation | CLI entry point |
| `t_spec` | `hex-cli/src/pipeline/workplan_phase.rs` | Loads behavioral specs from `docs/specs/` |
| `t_plan` | `hex-cli/src/pipeline/workplan_phase.rs` | Parses workplan JSON into `WorkplanData` |
| `t_worktree` | `scripts/feature-workflow.sh setup` | Creates git worktrees per adapter boundary |
| `t_fork` | `supervisor.rs` line ~1144 | `let max_tier = workplan.steps.iter().map(s.tier).max()` |
| `t_code_tN` | `supervisor.rs::run_tier()` line ~1227 | Objective loop: evaluate, fix, re-evaluate |
| `t_gate_*` | `supervisor.rs` line ~1938-1953 | `gate.blocking` check on `pre_validate` phase |
| `t_validate` | Agent role `validation-judge` | Returns PASS/FAIL verdict |
| `t_integrate` | `scripts/feature-workflow.sh merge` | Merge worktrees in dependency order |
| `t_finalize` | `scripts/feature-workflow.sh cleanup` | Remove worktrees, update swarm status |
| `P_end` | HexFlo `swarm_complete` reducer | Swarm status → "completed" |

---

## Known Gaps

1. **No failure/retry sub-net.** The net models the happy path. The retry
   loop (`MAX_ITERATIONS` in `run_tier`) and escalation path (inbox
   notification) are not encoded. A complete model would add a retry arc
   from each BLOCKING gate back to its input place, with a counter that
   eventually forces an abort transition.

2. **Within-tier parallelism is not modeled.** The net treats each
   `t_code_tN` as a single transition. The actual supervisor dispatches
   multiple agents within a tier (one per workplan step) with fork/join
   semantics. Modeling this requires parameterizing the sub-net by workplan
   step count, which makes soundness depend on the workplan.

3. **Worktree creation is assumed to succeed.** The net does not model
   `git worktree add` failure. In practice, worktree creation can fail
   (disk full, branch already exists), which would leave the token stuck
   at `P_plan`.

4. **No TLA+ encoding yet.** This net is specified in prose and ASCII art.
   A TLA+ encoding would make the soundness proof machine-checkable. For a
   sequential state machine this simple, enumeration suffices, but the
   within-tier fork/join sub-net would benefit from TLC.

# ADR-2604151430 — Parallel Worktree-Isolated Drain

**Status:** Proposed
**Date:** 2026-04-15
**Related:** ADR-2604150100 (worktree merge fast-forward guard), ADR-2604150130 (worktree cleanup safety), ADR-2604141400 (sched evidence-guard), ADR-2604151200 (idle research swarm), ADR-2604151330 (per-project queue isolation), ADR-2604151400 (queue list show running)

## Context

The sched daemon currently drains the queue **serially** — exactly one workplan task in flight at any time. Observed live in this session:

```
Queue: 1 running ▶ · 2 pending ⤵
Current: ▶ f0b3fc6a (workplan) docs/workplans/wp-brain-string-cleanup.json
```

The other two pending workplans (`f4f1e480` q-report, `b395249f` per-project queue) wait. The user asked: *"can't they be run in separate worktrees?"*

Yes — and the infrastructure to do it already exists:

- `hex worktree create/list/merge/cleanup` are first-class commands
- ADR-2604150100/0130 already enforce safety on merge (no-fast-forward-loss) and cleanup (no-killing-active-work)
- The CLAUDE.md feature-development workflow is already built around per-adapter worktrees with parallel agents
- HexFlo + the existing per-source-file conflict-avoidance pattern already encode the rule "parallelize by file boundary, serialize by file overlap"

The serial daemon is leaving idle CPU + idle local-LLM bandwidth on the table whenever the queue depth is ≥ 2. For independent workplans (touching disjoint files), there is no reason not to drain them concurrently.

## Decision

Add **parallel worktree-isolated drain** to the sched daemon, gated by a concurrency cap, file-overlap detection, and tier-aware resource throttles.

### Concurrency model

```
sched daemon
  ├─ slot 0: worktree feat/sched-drain/<task-id-1>/  ← drains task A
  ├─ slot 1: worktree feat/sched-drain/<task-id-2>/  ← drains task B
  ├─ slot 2: (idle)
  └─ slot 3: (idle)
```

- `max_parallel_drains` config (default **3**, max **8**); persisted in `<workspace_root>/.hex/project.json` under `sched.max_parallel_drains`.
- Each in-flight task gets its own git worktree at `feat/sched-drain/<task-id>/`. Branch name: `sched/<task-id>`.
- On task completion: `hex worktree merge` (the integrity-verified path per ADR-2604150100), then `hex worktree cleanup` (per ADR-2604150130).
- On task failure: worktree stays for forensics. A separate `hex sched gc-worktrees` command (added as part of this workplan) reaps failed-task worktrees older than N days.

### File-overlap detection (the conflict gate)

Before promoting a `pending` task to a `running` slot, the daemon:

1. Collects the union of `phases[].tasks[].files[]` from the candidate workplan
2. Compares against the union of `files[]` from every currently-running workplan
3. If any file overlaps → keep the task `pending` and pick the next candidate
4. If no overlap → promote, create worktree, dispatch

This is a static check from the workplan JSON. It's conservative (false positives possible if a file is in `files[]` but never written), but correctness > throughput. Cheap to compute; runs every tick.

### Tier-aware resource throttles

Local Ollama bandwidth is finite. Without a throttle, three T2.5 workplans landing simultaneously would saturate the GPU and slow every dispatch.

```
Tier  Default cap (concurrent in-flight workplans)
T1    max_parallel_drains       (no throttle — fast, cheap)
T2    max_parallel_drains       (no throttle — codegen sweet spot)
T2.5  max_parallel_drains_t25   (default 1; tunable)
T3    max_parallel_drains_t3    (default 1; frontier API rate limits)
```

Tier of a workplan is computed as `max(strategy_hint_tier across all tasks)`. So a workplan that contains any `inference` strategy hint counts as T2.5 and competes for the T2.5 quota.

### Failure isolation

A panicking child process or a workplan that exits non-zero affects only its own slot. The daemon catches the failure, marks the task `failed` with the existing evidence-guard semantics (ADR-2604141400), leaves the worktree for forensics, and frees the slot.

`max_failures` (existing daemon flag) becomes per-slot rather than global. Rationale: one bad workplan should not pause the whole daemon.

### Backward compatibility

Setting `max_parallel_drains: 1` in `.hex/project.json` recovers the current serial behaviour exactly. Default is 3 — slight risk of surprising existing users, mitigated by the file-overlap gate (which serializes anything that could conflict).

## Consequences

**Positive.**
- Throughput scales with disjoint workplans up to `max_parallel_drains`. Empirically a 3-task queue of independent workplans should finish in ~1× the time of the slowest task instead of ~3×.
- Failure isolation prevents one bad workplan from blocking the queue.
- File-overlap gate makes safe parallelism the default — operators don't have to think about which workplans are safe to run together.
- Reuses existing battle-tested worktree machinery + safety ADRs.

**Negative.**
- Higher peak resource use (CPU, disk for worktrees, RAM for child processes). Mitigated by the cap.
- More complex daemon state (a slot table instead of a single current_task pointer). Mitigated by P1.3 unit-test coverage.
- Worktree disk overhead — each worktree is a full checkout. On large repos this matters. Mitigated by aggressive cleanup-on-success.

**Risks.**
- File-overlap gate is conservative-static, not dynamic. A workplan can in principle write to a file not listed in `files[]`. Mitigation: post-merge `cargo check --workspace` build gate (already enforced by `hex worktree merge`) catches the wreckage; the sched daemon then marks the task `failed` and operators see the conflict cleanly.
- Multiple worktrees + a dirty main can stress git. Mitigation: pre-flight check that main is clean before promoting any task to running; if dirty, daemon logs a warning and waits.

## Non-goals

- **Not implementing dynamic file-write tracking.** Static `files[]` is the contract. If workplans lie about their files, that's a workplan-author bug.
- **Not parallelizing tasks within a single workplan.** Phase serialization stays as-is — that's a separate orchestration concern.
- **Not auto-merging from worktrees.** Merge happens only on full workplan success after evidence verification.
- **Not changing the queue ordering policy.** Still FIFO with priority, just with multiple slots.

## Implementation

See `wp-sched-parallel-worktree-drain.json`.

# ADR-2604291320 — Memory Health Swarm

**Status:** Accepted (2026-05-05)
**Date:** 2026-04-29
**Supersedes:** —
**Related:** ADR-2604151200 (idle-research swarm), ADR-2604142345 (insight routing), ADR-2604141400 (brain queue swarm-lease)

## Context

Memory (`hex memory search`, backed by SpacetimeDB `hexflo-coordination` module) accumulates state from every workplan execution, brain-task, and swarm operation. A search for `work` returns 335 entries (86 brain-task, 249 workplan task outcomes). Many are stale:

- **Failed brain-tasks** from timeouts, inference stalls, superseded workplans
- **Workplan task outcomes** from reconciled/completed workplans still marked in-progress or failed in memory
- **Orphaned swarm state** from interrupted sessions

The user asked: *"we need an active swarm loop to be assessing, categorizing, reporting and executing against this"* — referring to the memory health, not code health (idle-research swarm, ADR-2604151200).

**Current state:**
- Memory grows unbounded — no cleanup, no reconciliation
- `hex plan reconcile` fixes workplan JSON on disk but doesn't update memory entries
- Failed brain-tasks sit forever — no retry/archive/escalation logic
- No visibility into memory health (stale ratio, growth rate, failure categories)

**Why this matters:**
- Stale memory pollutes search results, slows queries
- Failed tasks that SHOULD retry (transient errors) never do
- Failed tasks that SHOULD NOT retry (duplicate/superseded work) waste tokens
- No feedback loop: same failure patterns repeat because memory doesn't learn

## Decision

Add a **memory-health swarm** that runs on a fixed interval (default: 1 hour, configurable via `.hex/project.json` → `memory.health_check_interval_h`). The swarm:

### Phase 1: Assessment

Run deterministic checks (zero-cost):

1. **Staleness detection**
   - Brain-tasks: `status=in_progress` with `leased_until` > 2h ago → stale
   - Brain-tasks: `status=failed` → categorize by error pattern
   - Workplan outcomes: `workplan:*:task:*:outcome` where workplan JSON shows task as `done` → memory drift

2. **Growth metrics**
   - Total memory entries by category (brain-task, workplan, swarm)
   - Growth rate: entries added in last 24h / last 7d
   - Failure rate: failed brain-tasks / completed brain-tasks (7d window)

3. **Failure pattern mining**
   - Group failed brain-tasks by `result` field (timeout, inference stall, compile error, etc.)
   - Count occurrences per pattern
   - Identify repeat failures (same payload, different task ID)

### Phase 2: Categorization

For each failed brain-task, assign action:

| Category | Condition | Action |
|---|---|---|
| **Transient** | `result` contains "timeout", "connection refused", "503" | Retry (re-enqueue) |
| **Superseded** | Workplan file no longer exists or marked complete on disk | Archive (mark `archived: true` in memory, don't delete) |
| **Compile blocker** | `result` contains "cargo check failed", "type error" | Escalate (create ADR draft "fix compile blocker before retry") |
| **Inference stall** | `result` contains "inference task failed", "empty response" | Escalate + retry with higher tier |
| **Duplicate** | Same payload as completed task in last 7d | Archive |
| **Unknown** | None of above | Log for human review |

### Phase 3: Reporting

Generate `docs/analysis/memory-health-YYYYMMDD-HHMM.yaml`:

```yaml
timestamp: "2026-04-29T13:20:00Z"
total_entries: 335
categories:
  brain_task: 86
  workplan_outcome: 249
stale_count: 23
failed_brain_tasks: 20
failure_patterns:
  - pattern: "timeout sweep"
    count: 8
    action: retry
  - pattern: "inference task failed"
    count: 5
    action: escalate
actions_taken:
  retried: 8
  archived: 10
  escalated: 2
growth_metrics:
  entries_24h: 15
  entries_7d: 89
  failure_rate_7d: 0.23
```

Also render human-readable `.md` summary alongside.

### Phase 4: Execution

**Non-destructive by default:**
- Archive = add `archived: true` field, keep entry (for audit / debugging)
- Retry = self-enqueue new brain-task with same payload + note "retry of <old-id>"
- Escalate = write ADR draft under `docs/adrs/drafts/`, do NOT auto-accept

**Configurable cleanup:**
- `.hex/project.json` → `memory.cleanup.archive_after_days: 90` — delete archived entries older than N days
- `.hex/project.json` → `memory.cleanup.max_retries: 2` — archive after N retry attempts

### Phase 5: Integration

- Daemon: new `kind: memory-health` task, auto-enqueued every `health_check_interval_h`
- Dashboard: panel showing memory health metrics (total, stale %, failure rate, recent actions)
- CLI: `hex memory health` — runs assessment + prints report

## Consequences

**Positive:**
- Memory self-heals — transient failures get retried, dead tasks get archived
- Failure patterns surface early → ADR drafts prevent repeated issues
- Bounded memory growth → cleanup prevents unbounded SpacetimeDB bloat
- Continuous feedback loop → system learns from past failures

**Negative / risks:**
- Aggressive retry could amplify a systemic issue (e.g., inference backend down → 20 retries all fail)
  - **Mitigation:** max_retries cap + exponential backoff (retry intervals: 1h, 4h, 24h)
- Incorrect categorization could archive work that should retry
  - **Mitigation:** dry-run mode (`hex memory health --dry-run`) logs actions without executing
- Memory cleanup could delete evidence needed for debugging
  - **Mitigation:** archived entries persist for `archive_after_days` (default 90d), can be restored

**Open questions:**
- Should retry inherit the original task's tier, or always start at T1 (cheapest)?
  - **Decision:** inherit tier unless categorized as "inference stall" → escalate to T2.5/T3
- Should escalation create ADR drafts or just log findings?
  - **Decision:** create ADR drafts for compile blockers + inference stalls (high signal); log others for human review

## Non-goals

- **Not replacing workplan reconciliation.** `hex plan reconcile` fixes workplan JSON; this fixes memory state. Both are needed.
- **Not a full memory compaction strategy.** This handles task/swarm state; generic memory entries (arbitrary key-value) are out of scope.
- **Not real-time.** Runs on 1h interval, not on every memory write.

## Implementation

See `wp-memory-health-swarm.json`.

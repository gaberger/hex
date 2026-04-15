# ADR-2604151400 — `hex sched queue list` Shows Running Tasks

**Status:** Proposed
**Date:** 2026-04-15
**Related:** ADR-2604150000 (brain→sched rename), ADR-2604151330 (per-project queue isolation)

## Context

`hex sched queue list` currently renders only **pending** tasks:

```
Pending Brain Tasks
╭──────┬──────────┬────────┬──────────┬─────────╮
│ ID   │ Kind     │ Target │ Payload  │ Created │
╰──────┴──────────┴────────┴──────────┴─────────╯
```

Meanwhile `hex sched status` shows the in-flight task as a one-liner:

```
Queue:   1 running ▶ · 2 pending ⤵
Current: ▶ f0b3fc6a (workplan) docs/workplans/wp-brain-string-cleanup.json
```

The two views disagree about queue contents. An operator running `queue list` while a task is in flight gets the false impression that *only* the pending tasks exist — and may then conclude (wrongly) that the daemon isn't draining. This is the exact misread that just happened in-session: the user saw two pending tasks, asked "is the daemon running?", and the answer turned out to be "yes, and a third task is currently executing — it's just hidden from `queue list`."

## Decision

`hex sched queue list` shows tasks in **all non-terminal states** by default: `running`, `pending`, and `failed-retrying` (if/when added). Terminal states (`completed`, `failed`, `cancelled`) are hidden by default and surfaced via flags.

### Surface

```
hex sched queue list                          # Default: running + pending
hex sched queue list --all                    # Include completed + failed
hex sched queue list --status running         # Filter by status
hex sched queue list --limit 50               # Row cap (default 50)
hex sched queue list --json                   # Machine-readable
```

### Output

```
Sched Queue — 1 running, 2 pending, 12 completed (use --all to show)

  STATUS    ID         KIND      PAYLOAD                                CREATED   AGE
  ▶ running f0b3fc6a   workplan  docs/workplans/wp-brain-string-cle…    11:32     8m
  ⤵ pending f4f1e480   workplan  docs/workplans/wp-inference-q-repo…    11:39     1m
  ⤵ pending b395249f   workplan  docs/workplans/wp-sched-queue-per-…    11:46     0s
```

Title becomes `Sched Queue` (closes the brain→sched rename gap on this surface). Status column uses the same glyphs already used by `hex sched status` (`▶ ⤵`) for visual consistency.

## Consequences

**Positive.**
- Single-source-of-truth: `queue list` and `status` agree on what's in the queue.
- Eliminates the recurring "is the daemon stuck?" misread.
- Cheap consistency win — no new data, just a render change.

**Negative.**
- Operators who relied on `queue list` to mean "*pending* only" would see one extra row when a task is in flight. Mitigation: the row is clearly marked `▶ running` and is visually distinct.

## Non-goals

- Not redesigning the underlying queue model.
- Not adding completed-task history paging — that's `--all` for now.

## Implementation

See `wp-sched-queue-list-show-running.json`.

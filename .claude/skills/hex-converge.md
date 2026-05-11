---
name: hex-converge
description: Run one full self-improvement convergence cycle — discover, judge, act --apply, wait, learn, status — and report whether homeostasis improved
triggers:
  - converge the loop
  - run improver cycle
  - drive homeostasis
  - improve hex
  - tighten the loop
  - hex converge
---

# hex-converge — One Self-Improvement Convergence Cycle

**Use this skill when**: you want the improver to actually move the homeostasis score, not just preview hypotheses. This runs the full discover → judge → act --apply → wait → learn → status arc and reports the delta.

Background: ADR-2026-04-27-1100. The improver is a closed-loop RL system over the workplan corpus. Read-only subcommands (`discover`, `judge`) only *describe* drift; only `act --apply` and `learn` change anything.

## Step 1 — Snapshot starting state

```bash
hex sched improver status
```

Capture the score, the dominant detector, the Q-table mean reward, and the dead-letter count. This is the baseline.

## Step 2 — Confirm daemon is running

```bash
hex sched daemon-status
```

If not running, start it (the convergence depends on the daemon draining the queue):

```bash
hex sched daemon --background --interval 30
```

## Step 3 — Apply the top-N actions

```bash
hex sched improver act --apply
```

This enqueues the auto-mappable hypotheses as priority-tagged sched tasks. **Do not pass `--dry-run`** — that's the default operator-preview mode and won't move anything.

## Step 4 — Wait one daemon tick (plus a margin)

The daemon's interval is whatever it was started with (default 30s). Wait at least `interval + 5s` so the queued tasks have a chance to drain.

```bash
sleep 35
hex sched queue history | head -20
```

Look at the most recent rows: `completed` is good, `failed` means salvage work — see `feedback_salvage_after_sched_fail`.

## Step 5 — Observe outcomes (Q-table update)

```bash
hex sched improver learn
```

This reads the workplan corpus deltas since `act --apply` and updates `~/.hex/improver/q-table.json`. The mean reward should move; if it doesn't, the actions had no observable effect — that itself is a signal.

## Step 6 — Re-snapshot and report the delta

```bash
hex sched improver status
hex sched improver history | head -20
```

Report:
- Score delta (e.g. 38 → 47 = +9)
- Mean reward delta (e.g. -0.064 → -0.012 = +0.052)
- Dominant detector before vs after (e.g. ReconcileStrict 81 → 23)
- Any *new* dead-letter rows (failed actions the loop couldn't recover from)

## Convergence vs thrash

A healthy cycle moves the score up and shrinks the dominant detector. A **thrashing** cycle shows the same workplan IDs cycling `completed (auto-retried)` ↔ `failed` in queue history. If you see thrash:

1. Pull the failing workplan path from the history row.
2. Open the workplan and the most recent `auto-retried` salvage entry under `~/.hex/sched/`.
3. The fix is upstream of the loop — usually a malformed task or a missing build gate. Don't re-`act --apply`; fix the workplan and reconcile (`hex plan reconcile --all --update`) first.

## Common pitfalls

- **Running `act` without `--apply`** — that's preview-only. Symptom: status doesn't move.
- **Skipping `learn`** — Q-table doesn't update; next cycle picks the same losing actions.
- **Running before daemon is up** — actions enqueue but never drain.
- **Re-running mid-cycle** — wait the full interval + margin before re-snapshotting; otherwise the tasks haven't completed and the delta is meaningless.

## ARGUMENTS

No arguments required. Run with: `/hex-converge`

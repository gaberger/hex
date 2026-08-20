---
name: hex-enqueue-work
description: Convert outstanding work into a concrete sched-queue entry — workplan path or shell command — and verify it's accepted by the sched guard
triggers:
  - enqueue this
  - queue this work
  - hex enqueue
  - put this on the queue
  - schedule this work
  - hex brain enqueue
---

# hex-enqueue-work — Enqueue Outstanding Work for the Sched Daemon

**Use this skill when**: you have outstanding work the autonomous loop should handle but isn't auto-discovering, or when CLAUDE.md rule #1 ("enqueue, never defer") applies. The autonomous-operation rule is explicit: any work that won't ship in this turn goes on the queue *now*, not "next session".

The sched guard rejects empty/stub work — `echo FIXME` shell commands, `null` payloads, drafts without phases. The skill enforces this *before* enqueueing so the queue stays clean.

## Step 1 — Decide the artifact type

| Work shape | Artifact |
|---|---|
| Feature-sized, multi-phase, cross-adapter | **Workplan JSON** in `docs/workplans/` |
| Single shell command (build, lint, format) | **Shell** payload |
| Ambiguous / not yet actionable | **ADR or TODO** — do NOT enqueue stubs |
| Repeating analysis (e.g. `hex analyze`) | **Shell** payload, optionally on a cron |

If the work is "not yet actionable", stop. CLAUDE.md rule #10: no `echo FIXME` stub tasks. Write an ADR or a TODO comment instead.

## Step 2a — Workplan path

If the work has phases and tasks, draft a workplan first. Either auto-invoke the T3 path (just describe the prompt naturally) or write the JSON yourself in `docs/workplans/wp-<slug>.json` following the schema in the `workplan-format` skill.

```bash
hex plan validate docs/workplans/wp-<slug>.json
```

If validation fails, fix the JSON before enqueueing. The sched guard runs the same validator and will reject malformed workplans.

```bash
hex sched enqueue workplan docs/workplans/wp-<slug>.json
```

## Step 2b — Shell command

```bash
hex sched enqueue shell -- "<command>"
```

Quote the command so the shell doesn't expand variables locally. The sched guard rejects:

- Empty commands
- Commands containing only `echo`/`printf`/`true`/`:`
- Commands that don't reference any verifiable artifact

If the guard rejects, the failure mode is "stub task" — fix the command to do real work, or move the intent to an ADR.

## Step 3 — Verify acceptance

```bash
hex sched queue list
```

The new task should appear with status `pending`. If it doesn't, check the rejection reason:

```bash
hex sched queue history | head -5
```

A guard rejection shows up as `failed` with a reason in the result-tail.

## Step 4 — Wait for drain (optional)

If you want to confirm execution rather than just acceptance, wait one daemon interval and check history:

```bash
sleep 35
hex sched queue history | head -10
```

`completed` = ran cleanly. `failed` = guard tripped on the result; see `/hex-salvage`.

## Cron-style repeating work

Some work (analysis sweeps, reconcile passes) makes sense to run on a schedule, not once. Use the schedule skill for those — `/schedule` integrates with the same sched daemon and supports cron expressions.

## Common pitfalls

- **Enqueueing stubs** — `hex sched enqueue shell -- "echo TODO"` is rejected by design. Don't try to work around it; the rejection is correct.
- **Skipping `plan validate`** — the sched guard runs the same validator; pre-validating saves a queue slot.
- **Enqueueing into a stopped daemon** — tasks queue but never drain. Check `hex sched daemon-status` first.
- **Treating "queued" as "done"** — verify with `queue history` after one interval. Queueing alone doesn't ship anything.

## ARGUMENTS

Pass the work description: `/hex-enqueue-work <workplan-path-or-shell-command>`

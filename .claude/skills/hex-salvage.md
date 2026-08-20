---
name: hex-salvage
description: Diagnose and salvage a failed sched task — inspect git, build state, and partial progress before re-enqueueing
triggers:
  - salvage failed task
  - workplan failed
  - sched task failed
  - hex salvage
  - recover failed workplan
  - investigate sched failure
---

# hex-salvage — Salvage a Failed Sched Task

**Use this skill when**: `hex sched queue history` shows a `failed` row and you need to decide whether the work itself failed or whether the sched guard prematurely flipped real progress to `failed`.

Background: the sched guard is conservative — agents that bundle commits, skip the per-task commit step, or write to the index without committing get marked `failed` even when their changes are sound. Re-enqueueing without inspecting first wastes inference and can clobber salvageable work. See `feedback_salvage_after_sched_fail` in memory.

## Step 1 — Identify the failed task

```bash
hex sched queue history | head -30
```

Capture: task id, kind, payload (workplan path), and timestamp. The `Result-Tail` column usually shows the guard's complaint (e.g. `dir=.` with no diff, missing commit, etc).

## Step 2 — Pull the full result blob

```bash
hex sched queue history --json 2>/dev/null | jq '.[] | select(.id=="<task-id>")'
```

If `--json` isn't supported on your build, the result file lives at `~/.hex/sched/results/<task-id>.json`. Read the full stderr and stdout — the guard's reason should be explicit.

## Step 3 — Inspect git for unrecorded progress

This is the load-bearing step. The guard's complaint is usually "no commit" or "diff doesn't match expected files." But the agent may have:

- staged files without committing
- written files without staging
- committed under an unexpected message format

```bash
git status --short
git diff --stat HEAD
git log --oneline --since="<task started>" -- <workplan files>
```

If the workplan touches specific files, check each:

```bash
git log --all --oneline -- <file>
```

## Step 4 — Verify build state

If the task's workplan involves a build, run the appropriate gate **before** any salvage commit. Salvaging broken code is worse than re-running the task.

```bash
# Rust crates
cargo check --workspace

# TypeScript examples
cd <example-dir> && npx tsc --noEmit
```

## Step 5 — Decide: salvage, re-enqueue, or abandon

| Situation | Action |
|---|---|
| Files written, build clean, no commit | **Salvage**: stage + commit in one shell call (see main-branch-concurrency note), then mark task done |
| Files written, build broken | **Don't salvage**: revert with `git restore`, fix the workplan, re-enqueue |
| No diff, no progress | **Re-enqueue** the original workplan |
| Workplan itself malformed | **Don't re-enqueue**: fix the JSON, reconcile (`hex plan reconcile --all --update`), then enqueue |

## Step 6 — Salvage commit (if applicable)

Combine staging and committing into a single shell call to avoid the brain daemon resetting the index between them:

```bash
git add <files> && git commit -m "salvage: <task-id> — <one-line summary>"
```

## Step 7 — Mark the sched task complete (only if salvage succeeded)

If the work is genuinely done after salvage, you don't need to re-enqueue. The next improver cycle will detect the workplan as fulfilled and stop firing on it. Verify:

```bash
hex sched improver discover --once | grep -i <workplan-id>
```

If the hypothesis is gone, the loop converged. If it's still there, the discover oracle disagrees with your salvage — investigate before re-applying.

## Common pitfalls

- **Re-enqueueing without inspecting** — wastes inference and can blow away local diffs the agent already produced.
- **Staging in one shell, committing in another** — the brain daemon may reset between them. Always combine.
- **Salvaging broken code** — always run the build gate before committing.
- **Trusting the result-tail** — the guard's complaint is a hint, not the truth. Always corroborate with `git status` and a build.

## ARGUMENTS

Pass the failed task id or workplan path: `/hex-salvage <task-id-or-workplan-path>`

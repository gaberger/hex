# ADR-2604151330 — Per-Project Sched Queue Isolation

**Status:** Proposed
**Date:** 2026-04-15
**Related:** ADR-2604150000 (brain→sched rename), ADR-2604141400 (sched evidence-guard), ADR-2604151200 (idle research swarm), ADR-2604142200 (reconcile evidence verification)

## Context

The sched queue is currently persisted to a single global file:

```
hex-cli/src/commands/sched.rs:44
PathBuf::from(home).join(".hex").join("brain-state.json")
```

This means every hex project on the machine reads and writes the **same queue file**. Symptoms observed in the field:

```
$ cd ~/work/hex-intf && hex sched queue list
  pending  workplan  docs/workplans/wp-brain-string-cleanup.json   (this project — fine)
  pending  workplan  docs/workplans/wp-foo-from-other-project.json (NOT in this repo — leak)
```

The daemon then tries to drain the cross-project task, fails the file-not-found check, marks it failed, and the operator in the wrong project sees alarming output for work they didn't author. This is also a soft data-leak: project B can see workplan filenames + descriptions from project A, which may be private.

The same global-file pattern affects the daemon PID file (`~/.hex/brain-daemon.pid`, line 1357), which means **only one sched daemon can run on a machine** — a second project trying to start its own daemon either silently no-ops or kills the other project's daemon. Both are wrong.

The user explicitly flagged it: *"another project should not know about workitems from another project."*

## Decision

**Move all sched per-project state under `<workspace_root>/.hex/sched/`** so it is project-scoped, naturally git-ignored, and survives no other project's actions.

### New layout

```
<workspace_root>/.hex/sched/
  queue.json              # Replaces ~/.hex/brain-state.json
  daemon.pid              # Replaces ~/.hex/brain-daemon.pid
  daemon.log              # Daemon stdout/stderr
  last_research_sweep     # Per-project sweep throttle (ADR-2604151200)
```

`workspace_root` is resolved via the existing `find_workspace_root()` helper (walks up from cwd looking for `.hex/` or `.git/`). If neither is found, the CLI errors with a clear "not in a hex project" message rather than silently falling back to `~/.hex/`.

### Daemon scope

A sched daemon is bound to **one** workspace_root, recorded in its PID file as `{ pid: <int>, workspace_root: <abs path>, started_at: <iso8601> }`. Running `hex sched daemon` in a different project starts a separate daemon. Each daemon only sees its own project's queue.

This matches the user's mental model: projects are independent, so their queues are independent.

### Cross-project visibility (out of scope here, but designed-in)

For users who *do* want a fleet view across projects, a follow-up ADR will add `hex sched fleet` that aggregates queues by reading each project's `<workspace_root>/.hex/sched/queue.json` (discovered via the existing project-registry under `~/.hex/projects/`). This ADR explicitly does not implement that — it only fixes the leak.

### Migration

On first run after upgrade:

1. If `<workspace_root>/.hex/sched/queue.json` does not exist AND `~/.hex/brain-state.json` does:
   - Load `~/.hex/brain-state.json`
   - For each task whose `kind=workplan` resolves to a file under `<workspace_root>/`: copy it to the new per-project queue
   - For each other task: leave it in `~/.hex/brain-state.json` (it belongs to a different project)
   - Write a one-time migration marker `<workspace_root>/.hex/sched/.migrated-from-global`
2. Print a one-line stderr notice: *"Migrated N tasks from ~/.hex/brain-state.json to <workspace_root>/.hex/sched/queue.json. Run `hex sched gc-global` to clean up the legacy file once all projects have migrated."*
3. Add a new `hex sched gc-global` command that errors if any tasks remain unclaimed (so the user explicitly chooses to delete cross-project state).

### Orphan-task GC (secondary fix)

Even within a project, a workplan task can become orphaned if the JSON file is deleted or moved. Add a daemon precondition: before draining a `kind=workplan` task, check the file exists and is readable relative to workspace_root. If not, mark the task `failed` with reason `workplan_file_missing: <path>` rather than running the subprocess. This is a much faster failure mode than the current "exec subprocess, get exit 1, blame the user."

## Consequences

**Positive.**
- Queues are properly isolated; no more cross-project leaks.
- Multiple projects can run independent sched daemons concurrently.
- `.hex/sched/` is a natural unit for `.gitignore` (already covered by existing `.hex/` ignore in most projects).
- Orphan-task detection turns a confusing late failure into a clear early one.

**Negative.**
- Breaking change to disk layout. Mitigated by automatic migration + the `gc-global` escape hatch.
- Operators who relied on the global queue as a de-facto fleet view lose that. Mitigated by the planned `hex sched fleet` follow-up.
- A second sched daemon per machine doubles memory baseline. Acceptable — daemons are small (<50 MB), and this matches per-project hex-nexus daemons already.

## Non-goals

- **Not implementing fleet-view aggregation.** That is a follow-up ADR.
- **Not changing queue semantics** (FIFO, evidence-guard, etc. — same as today, just per-project).
- **Not migrating SpacetimeDB-backed coordination** (HexFlo swarms, fleet tables) — those are *intentionally* cross-project and stay where they are.

## Implementation

See `wp-sched-queue-per-project.json`.

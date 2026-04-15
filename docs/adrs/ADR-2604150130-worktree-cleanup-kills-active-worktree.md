# ADR-2604150130: `hex worktree cleanup` destroys active agent worktrees whose branch has not yet diverged from main

<!-- ID format: YYMMDDHHMM — 2604150130 = 2026-04-15 01:30 local -->

- **Status**: Proposed
- **Date**: 2026-04-15
- **Depends on**: ADR-2604150100 (related worktree-merge drop-commits bug)
- **Relates to**: `feedback_enforce_worktrees.md`, `feedback_use_hex_worktree_merge.md`

## Context

During the 2026-04-14 autonomy-fix session, a background agent (E) was dispatched to a fresh isolated worktree at `.claude/worktrees/agent-a3358986`. Roughly 3–4 seconds after dispatch, before the agent had committed any work, `hex worktree cleanup --force` was run from the orchestrator session with the intent to reap already-merged historical worktrees.

The tool reported:

```
Found 1 merged worktree(s):
  – worktree-agent-a3358986 (/Volumes/.../agent-a3358986)
✓ Removed /Volumes/.../agent-a3358986 (branch worktree-agent-a3358986)
```

It destroyed Agent E's live working directory while the agent was mid-run.

**Why it classified "merged"**: the worktree branch was created at main's HEAD. Until the agent commits its first change, `branch tip == main tip`, which is indistinguishable from "branch has been fully merged into main" using a naïve ancestor check. The cleanup tool treated the two states identically.

**Outcome**: Agent E actually DID land its commits (they made it to main via git's direct ref updates before the directory deletion — a fortunate race), but this is accidental survival. A slower agent, or one holding uncommitted work in the working tree, would have lost everything. The orchestrator then dispatched a duplicate Agent F to recover work that was already done — wasted effort + potential for merge conflicts.

This bug has the same root cause as ADR-2604150100 (`hex worktree merge` fast-forward silent-drop): **the tool does not distinguish "ancestor of main" from "active workspace not yet diverged"**. Both bugs stem from "is-ancestor" being insufficient to prove safety.

## Decision

`hex worktree cleanup` must refuse to remove any worktree that shows **any sign of active work or unreadiness**. Specifically:

### 1. Cleanup preconditions (all must hold)

For each candidate worktree, require:

1. **Branch has strictly diverged from main** — `git rev-list --count main..<branch>` > 0 **AND** every commit on that branch is an ancestor of main (fully merged). If the branch has zero commits ahead of main, it's not "merged" — it's "hasn't started yet." Skip.
2. **Working tree is clean** — `git -C <worktree> status --porcelain` returns empty. Any uncommitted work means the agent is in-flight.
3. **Index is clean** — no staged changes.
4. **No process has the worktree cwd open** — check via `lsof +D <worktree>` or at minimum `fuser -v <worktree>`. If any process holds it (typically the agent subprocess), skip.

If all four hold, cleanup is safe. Otherwise, refuse with a clear line naming which precondition failed.

### 2. Tri-state output instead of boolean

Today the output is binary: "merged → removed" vs "not-merged → kept." Make it tri-state:

```
candidate worktree-agent-abc...
  merged      : yes (3 commits, all ancestors of main)
  working tree: clean
  index       : clean
  process hold: none
  → safe to remove

candidate worktree-agent-def...
  merged      : N/A (branch has no commits ahead of main — not yet started)
  → SKIP (unready)

candidate worktree-agent-ghi...
  merged      : yes
  working tree: dirty (2 modified files)
  → SKIP (in-flight)
```

Silent success is the bug. Agents reading the output must be able to tell why a worktree was kept.

### 3. Dry-run by default, `--force` plus explicit list to bypass

Today `--force` is required for actual removal but applies globally to every "merged" worktree. Change to:

- Bare `hex worktree cleanup` → dry-run listing (as today).
- `hex worktree cleanup --force` → remove **only** the worktrees that pass all four preconditions. Print the full tri-state per candidate.
- `hex worktree cleanup --force --name <branch>` → remove one specific worktree, still enforcing preconditions (for targeted reaping without touching siblings).

### 4. Heartbeat / claim file (optional, future)

Background agents could write a `~/.hex/worktree-claims/<branch>.toml` with a pid + heartbeat timestamp. Cleanup checks for a live claim and skips the worktree if the claim is fresh (<60s). The agent's parent can remove the claim on normal completion; crashed-agent claims go stale and become eligible for cleanup after the sweeper interval. Not required for the core fix but closes the remaining race.

## Consequences

**Positive**: Active agent worktrees become safe from concurrent cleanup — even when an orchestrator is juggling multiple spawn/reap operations. The same observability discipline as ADR-2604150100 (tri-state output) applies. Duplicate-work waste (orchestrator dispatching a recovery agent for work that was already done) disappears.

**Negative**: Cleanup becomes slightly slower — one `git status` + one `lsof` per candidate. Negligible unless dozens of worktrees exist; the existing 20+ backlog is a one-time clear.

**Migration**: Existing "merged + clean + unheld" worktrees continue to be reaped. Only "zero-commits-ahead" and "dirty-working-tree" worktrees change behavior. No breaking changes to invocations.

## Test plan

- Regression test: create a fresh worktree, run `hex worktree cleanup --force` immediately, assert the worktree survives with `merged: N/A (no commits ahead)` in output.
- Dirty-tree test: create a worktree, write a file, don't commit, run cleanup; assert survival with `working tree: dirty`.
- Process-held test: create a worktree, `cd` into it in a held shell, run cleanup; assert survival with `process hold: <pid>`.
- Happy path: create a worktree, commit something, merge it to main, run cleanup; assert removal with full green tri-state.
- Replay of 2026-04-14 incident using a fixture repo: verify the new tool would have preserved Agent E's worktree.

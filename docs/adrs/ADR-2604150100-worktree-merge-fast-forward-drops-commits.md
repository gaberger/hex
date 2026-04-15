# ADR-2604150100: `hex worktree merge` silently drops commits when worktree branch diverged before merge

<!-- ID format: YYMMDDHHMM — 2604150100 = 2026-04-15 01:00 local -->

- **Status**: Proposed
- **Date**: 2026-04-15
- **Depends on**: ADR-2604131930 (original worktree-merge integrity claim)
- **Relates to**: `feedback_use_hex_worktree_merge.md`

## Context

During the 2026-04-14 autonomy-fix session, three background agents (A, B, C) ran in parallel against `main`. Timeline:

1. Agent A committed directly on `main` (did not use its assigned worktree): HEAD `c15f62b7` → `becdacd1` (+4 commits)
2. Agent B worked in worktree branch `worktree-agent-a1c9441e`, branched at `becdacd1` (correctly). Committed 3 insights commits (`5946be82`, `e3c4f7e3`, `e7682b51`).
3. Agent B's worktree was merged via `hex worktree merge agent-a1c9441e --force`. Success. Main HEAD advanced to `e7682b51`. `git log -8` confirmed.
4. Agent C, launched BEFORE Agent B's merge landed, worked in worktree branch `worktree-agent-a6324814`, branched at `becdacd1` (Agent C's start point — before Agent B's merge). Committed 4 observability commits.
5. Agent C's worktree was merged via `hex worktree merge agent-a6324814 --force`. Success reported.
6. **Post-merge `git log` showed Agent B's 3 commits had disappeared from main.** Main now read: `aae367a2 → 26dd82c5 → 34c74884 → e6478f2a` (Agent C) on top of `becdacd1` (Agent A). Agent B's commits were only reachable via `git log --all`.

Root cause: `hex worktree merge` performs a fast-forward from main's current HEAD to the worktree-branch HEAD when the two share an ancestor and the worktree branch is not behind. Because Agent C's branch was based on `becdacd1` (predating Agent B's merge), fast-forwarding to Agent C's tip **rewound** main from `e7682b51` back to `becdacd1`, then to C's 4 commits — dropping B's 3 commits from main's first-parent history.

Recovery was possible (`git cherry-pick 5946be82 e3c4f7e3 e7682b51` with one trivial Cargo.toml conflict), but the tool gave zero warning that commits were being dropped.

This is the exact failure mode the `feedback_use_hex_worktree_merge.md` rule was created to prevent (raw `git checkout <branch> -- <file>` silently dropping code from other worktrees) — but the *integrity-verified* merge tool itself has the same failure mode when the worktree branch predates an intervening merge.

## Decision

`hex worktree merge` must detect and refuse to complete a merge that would drop commits. Specifically:

### 1. Pre-merge divergence check

Before the merge, compute:

```
BASE = git merge-base <worktree-branch> main
AHEAD_MAIN = git rev-list --count BASE..main     # commits main has that worktree does not
AHEAD_WT   = git rev-list --count BASE..<branch> # commits worktree has that main does not
```

- `AHEAD_MAIN == 0` → fast-forward is safe (main hasn't advanced). Proceed as today.
- `AHEAD_MAIN > 0 && AHEAD_WT > 0` → **real three-way merge required**. Refuse fast-forward. Perform a `git merge --no-ff` OR require explicit `--rebase` flag to rebase the worktree branch onto main first and re-run.
- `AHEAD_MAIN > 0 && AHEAD_WT == 0` → worktree branch is strictly behind. Nothing to merge. Warn and exit success.

### 2. Post-merge integrity check

After the merge, assert every pre-merge main commit is still reachable from the post-merge HEAD:

```
pre_main_head = <captured before merge>
git merge-base --is-ancestor pre_main_head HEAD || PANIC "worktree-merge dropped commits"
```

If the assertion fails, hard-error and leave `ORIG_HEAD` intact so the user can `git reset --hard ORIG_HEAD` to recover.

### 3. Surface the merge strategy used

Always print one line describing what actually happened:

```
worktree-merge: fast-forward  main {pre}→{post}  (+N commits, no divergence)
worktree-merge: three-way     main {pre}→{post}  (worktree +M, main was +N since base)
worktree-merge: no-op         worktree was strictly behind main
```

Silent success is the bug. Any agent reading the output should know which path was taken.

## Consequences

**Positive**: Parallel worktree merges become safe without manual re-verification. `git log --all` / `--first-parent` confusion no longer required to catch silent loss. The `feedback_use_hex_worktree_merge` rule actually holds.

**Negative**: Fast-forward merges become slower (extra rev-list counts + post-merge is-ancestor check). Negligible in practice — single-digit milliseconds on repos with <10K commits.

**Migration**: Existing worktree branches continue to merge cleanly because the new check only refuses fast-forwards that *would* drop commits. Worktrees that merge safely today continue to do so.

## Test plan

- Regression test: spawn two worktree branches off the same base; merge one; attempt to fast-forward-merge the other; assert the tool either rebases or three-way-merges, and that both sets of commits are reachable from the post-merge HEAD.
- Replay of the 2026-04-14 incident using a fixture repo: verify the new tool would have refused the fast-forward or performed a three-way merge, preserving B's commits.

# ADR-004: Git Worktrees for Parallel Agent Isolation

## Status: Accepted
## Date: 2026-03-15

## Context

In swarm mode, multiple LLM agents work on different adapters simultaneously. Agents need filesystem isolation so their file writes do not conflict. Standard git branches share a single working directory, making true parallel editing impossible.

## Decision

Each agent receives a dedicated **git worktree** linked to a feature branch. The `IWorktreePort` manages the lifecycle: create, merge, cleanup.

### Why Worktrees Over Branches

| Approach | Parallel FS Access | Conflict Risk | Agent Complexity |
|----------|-------------------|---------------|-----------------|
| **Branches (checkout)** | No — shared working dir | High — agents overwrite each other | Must serialize access |
| **Separate clones** | Yes | Low | Heavy disk/network cost |
| **Worktrees** | Yes — each has own dir | Low — isolated until merge | Lightweight, shares .git |

Worktrees give each agent a real filesystem path (`../hex-<task>`) while sharing the object database. Agents read and write freely without coordination locks.

### Ruflo Integration

The swarm coordinator maps each Ruflo task to a worktree:

1. `task_create("impl-cli-adapter")` registers the task
2. `IWorktreePort.create("feat/cli-adapter")` creates `../hex-cli-adapter/`
3. Agent spawns with `cwd` set to the worktree path
4. On completion: `task_complete` records the commit hash from the worktree

The `WorktreePath` value object carries both `absolutePath` and `branch`, ensuring the merge step always knows the source.

### Merge Strategy

1. **Rebase-first**: `git rebase main` in the worktree branch. If no conflicts, fast-forward merge to main.
2. **Fallback**: If rebase conflicts, abort rebase and create a merge commit. Record conflicts in `MergeResult.conflicts` for the integration agent to resolve.
3. **Ordering**: Merge worktrees in dependency order (leaf adapters first, then adapters with cross-dependencies).

### Cleanup Protocol

After successful merge:

1. Verify the merge commit exists on the target branch
2. `git worktree remove <path>` removes the filesystem directory
3. `git branch -d <branch>` deletes the feature branch
4. Ruflo task status updated to `completed`

On failure, worktrees are preserved for debugging. A periodic `git worktree prune` removes stale entries.

## Consequences

### Positive

- True parallel agent execution with zero coordination overhead
- Disk-efficient — worktrees share the git object store
- Clean audit trail — each agent's work is a discrete branch with commits

### Negative

- Worktrees consume disk space proportional to working tree size (not repo history)
- Merge conflicts still require an integration agent to resolve
- Platform-specific path handling needed (worktree paths are absolute)

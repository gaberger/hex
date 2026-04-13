# ADR-2604131930: First-Class Worktree Lifecycle Management

**Status:** Proposed
**Date:** 2026-04-13
**Drivers:** Worktrees are hex's primary isolation mechanism for parallel agent work, but their lifecycle is fragmented across shell scripts, hooks, agent internals, and manual git commands. Code was silently dropped during a merge because hex didn't own the full lifecycle. An AIOS must guarantee that every line of agent-generated code reaches main.

## Context

hex uses git worktrees (ADR-004) to isolate parallel agent work. Today the lifecycle is:

| Phase | Owner | Problem |
|-------|-------|---------|
| **Create** | Claude Code `isolation: "worktree"` | hex doesn't know the worktree exists |
| **Track** | None | No registration in HexFlo; `hex brief` can't report worktree status |
| **Scope** | `.hex/task.json` + pre-commit hook | Stale scope blocks commits (G1), no dynamic scope expansion |
| **Merge** | Manual `git checkout` or `hex worktree merge` | `git checkout` silently drops code from other worktrees |
| **Verify** | `hex worktree merge --force` (new) | Only runs if developer remembers to use hex's tool |
| **Cleanup** | `hex worktree cleanup` (new) | Only runs if developer remembers; stale worktrees accumulate |

### What went wrong (2026-04-13)

P1.2 agent added `Brief` to `main.rs`. P6 agent also edited `main.rs`. Manual `git checkout` of P6's `main.rs` silently dropped P1.2's additions. `hex brief` CLI was missing for the entire session. The merge integrity check (built same session) would have caught this — but it wasn't used because the merge was done manually.

### Root cause

hex doesn't own worktree creation, so it can't enforce worktree merge. Agents create worktrees via Claude Code's `isolation: "worktree"` parameter, which bypasses hex entirely. hex only sees the worktree after the fact, when `git worktree list` reveals branches it didn't create.

## Decision

### 1. hex worktree create — registered worktrees

All worktrees MUST be created via `hex worktree create`:

```bash
hex worktree create <name> [--task <task-id>] [--files <file1,file2>]
```

This:
1. Runs `git worktree add .claude/worktrees/<name> -b worktree-<name>`
2. Registers the worktree in HexFlo memory: `worktree:<name>` → `{branch, path, task_id, files, agent_id, created_at, status}`
3. Creates `.hex/task.json` in the worktree with the `files[]` scope
4. Returns the worktree path for the agent to use

When Claude Code's `isolation: "worktree"` is used, the hex `subagent-start` hook SHALL call `hex worktree create` automatically if the worktree isn't already registered.

### 2. hex worktree status — live tracking

```bash
hex worktree status
```

Shows all registered worktrees with:
- Agent assignment (who's working in it)
- Files being modified
- Last commit timestamp
- Overlap warnings (files shared with other active worktrees)

Feeds into `hex brief` and `hex pulse` for visibility.

### 3. hex worktree merge — integrity-verified merging

```bash
hex worktree merge --all --force
```

The merge command (already built) now includes a **hard integrity gate**:
- After merging, verifies every added line from every worktree is present on main
- If any code is missing → merge is BLOCKED with diagnostic output
- Only after integrity passes → `cargo check --workspace` gate
- Only after both pass → files are committed

For overlapping files (multiple worktrees edit same file), the merge command SHALL:
1. Detect the overlap
2. Use `git merge-file` (3-way merge) instead of `git checkout` (destructive overwrite)
3. If auto-merge fails → stop and report the conflict for manual resolution

### 4. Automatic lifecycle in workplan executor

When `hex plan execute` runs:
- **Phase start**: calls `hex worktree create` for each agent task
- **Agent spawn**: worktree path passed to agent
- **Agent complete**: auto-runs `hex worktree merge --verify` (integrity check only, no commit)
- **Phase gate**: after all agents in a phase complete, runs `hex worktree merge --force` (commit)
- **Phase cleanup**: `hex worktree cleanup --force` removes merged worktrees

### 5. Pre-bash guard for raw git checkout

The `hex hook pre-bash` handler SHALL detect `git checkout <worktree-branch> -- <file>` and block with:

```
hex: BLOCKED — use 'hex worktree merge' instead of 'git checkout' for worktree files.
Raw git checkout silently drops code from other worktrees (ADR-2604131930).
```

This makes the destructive path impossible, not just discouraged.

## Consequences

**Positive:**
- Every worktree is tracked — visible in brief, pulse, status
- Code can never be silently dropped during merges (integrity gate)
- Workplan executor owns the full lifecycle — no manual merge step
- File overlap detected at create time, not at merge time
- Pre-bash guard prevents the destructive git checkout pattern

**Negative:**
- Agents must use `hex worktree create` (or the hook must auto-register)
- 3-way merge for overlapping files is slower than git checkout
- Pre-bash guard adds ~10ms to every Bash command

**Mitigations:**
- Auto-registration in `subagent-start` hook handles Claude Code's `isolation: "worktree"`
- 3-way merge only triggers for overlapping files (rare with good task decomposition)
- Pre-bash guard only checks commands containing "git checkout" + a worktree branch name

## Implementation

| Phase | Description | Status |
|-------|------------|--------|
| P1 | `hex worktree create` with HexFlo registration | Pending |
| P2 | `hex worktree status` with overlap detection | Pending |
| P3 | 3-way merge for overlapping files in `hex worktree merge` | Pending |
| P4 | Auto-registration in `subagent-start` hook | Pending |
| P5 | Workplan executor auto-lifecycle (create → merge → cleanup) | Pending |
| P6 | Pre-bash guard blocking raw git checkout | Pending |
| P7 | Integration tests — full lifecycle with mock agents | Pending |

## References

- ADR-004: Git Worktrees for Parallel Agent Isolation
- ADR-2603241700: Worktree Enforcement in Agent Hooks
- ADR-2604131800: Last-Mile Self-Hosting Gaps (G4, G6, G7)
- Session 2026-04-13: `hex brief` CLI dropped by destructive git checkout merge

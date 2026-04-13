# ADR-2604131800: Last-Mile Self-Hosting Gaps — hex Must Use hex

**Status:** Proposed
**Date:** 2026-04-13
**Drivers:** Dog-fooding session exposed 7 friction points where hex's designed workflow breaks down in practice. Agent sessions bypass hex's own formalism because the tooling doesn't close the loop.

## Context

hex is an AIOS that enforces hexagonal architecture and coordinates swarm agents. But when hex is used to develop *itself*, several gaps force the operator to work around hex's own rules:

### Observed Gaps (2026-04-13 session)

| # | Gap | What happened | Root cause |
|---|-----|---------------|------------|
| G1 | **Stale `.hex/task.json` blocks commits** | Pre-commit hook rejected files from new task because task.json was scoped to prior task (P4.1 → ADR-2604131630) | No auto-reset when task context changes |
| G2 | **No `CLAUDE_SESSION_ID` → Path B dead** | Inbox notifications never arrived; executor queued to unknown agent | Session identity not established in MCP-hosted sessions |
| G3 | **Workplan status drift** | 3/4 P1 tasks showed "todo" but code already existed | No reconciliation between git state and workplan JSON |
| G4 | **Worktree merge conflicts** | 3 agents all edited `scaffolding.rs` from same base → triple conflict | Parallel worktrees on same file; no file-overlap detection at dispatch |
| G5 | **Brief output 117K chars** | `hex brief` returns full event history, unusable without subagent summarization | No pagination, truncation, or summary mode |
| G6 | **9 stale worktrees** | Worktrees from completed agents never cleaned up | No auto-cleanup after task completion |
| G7 | **Manual worktree merge** | Operator had to `git checkout <branch> -- <file>` for each worktree | No `hex worktree merge` command that handles dependency order |

### Related ADRs

- **ADR-004** (Git Worktrees) — defines isolation but not merge/cleanup lifecycle
- **ADR-048** (Session Agent Registration) — session identity exists but not for MCP-hosted sessions
- **ADR-2603241700** (Worktree Enforcement) — enforces worktrees but no file-overlap guard
- **ADR-2604051700** (Enforce Workplan Gates) — gates exist but status doesn't reconcile with git
- **ADR-2604131500** (AIOS Experience) — brief/pulse exist but brief is unbounded

## Decision

### G1: Auto-reset `.hex/task.json` on task context change

The `hex hook route` classifier (UserPromptSubmit) SHALL detect when the user's intent doesn't match the current `.hex/task.json` scope. When the prompt classifies as a new T2/T3 task, the hook SHALL:
1. Archive current task.json to `.hex/task-history/`
2. Create a new task.json with the inferred scope
3. Or clear task.json entirely if on `main` (not a worktree)

The pre-commit hook SHALL allow all files when `.hex/task.json` is absent (main branch, no active task).

### G2: Auto-establish session identity for MCP-hosted sessions

When `CLAUDE_SESSION_ID` is unset but `hex mcp` is serving tools, the MCP server SHALL:
1. Generate a deterministic session ID from `hostname + PID + timestamp`
2. Register the agent via `hex hook session-start` on first tool call
3. Store the session in the same `~/.hex/sessions/agent-{id}.json` format
4. Path B notifications will then target this agent correctly

### G3: Workplan status reconciliation with git

`hex plan status <file>` SHALL cross-reference each task's `files[]` against `git log` to detect:
- Files modified since workplan creation → task likely done
- `done_command` exits 0 → task confirmed done

A new command `hex plan reconcile <file>` SHALL update task statuses in the workplan JSON based on git evidence. This runs automatically at the start of `hex plan execute`.

### G4: File-overlap detection at agent dispatch

Before spawning a worktree agent, the executor SHALL check if any running agent's `files[]` overlaps with the new agent's `files[]`. If overlap detected:
- **Option A**: Serialize — queue the new agent until the conflicting one completes
- **Option B**: Merge scope — combine both tasks into one agent (same worktree)
- Default: Option A (serialize). Log a `tracing::warn!` with the overlapping files.

### G5: Brief pagination and summary mode

`hex brief` SHALL default to summary mode (under 2KB):
- Last 5 events per project (not all)
- Pending decisions (always full)
- Agent count + health score

Full mode available via `hex brief --full` or `hex brief --since <timestamp>`.

### G6: Auto-cleanup worktrees on task completion

When `hex_hexflo_task_complete` is called AND the task was executed in a worktree:
1. Check if the worktree branch has been merged to its base branch
2. If merged: `git worktree remove <path>` + `git branch -d <branch>`
3. If not merged: mark as "pending-merge" in HexFlo memory, surface in `hex brief`

`hex worktree stale` (existing) SHALL also trigger cleanup for worktrees older than 24h with no commits.

### G7: `hex worktree merge` command

New command: `hex worktree merge <feature-name>` SHALL:
1. List all worktrees for the feature
2. Detect file overlaps between worktrees
3. For non-overlapping worktrees: merge in dependency tier order (domain → ports → adapters)
4. For overlapping worktrees: checkout the most recent version of each file (by commit timestamp)
5. Run `cargo check --workspace` as gate before committing
6. Clean up merged worktrees

## Consequences

**Positive:**
- hex can develop itself without operator workarounds
- Session identity works in all environments (Claude Code, MCP, standalone)
- Workplan status reflects reality, not stale JSON
- No more triple-conflict merges from parallel agents
- Brief is usable without subagent summarization

**Negative:**
- File-overlap detection adds latency to agent dispatch (~50ms per check)
- Auto-reset of task.json could surprise developers mid-task
- Reconciliation heuristic (git log → task status) may produce false positives

**Mitigations:**
- Overlap check is a hashset intersection — constant time in practice
- Task.json reset only triggers on main branch with explicit T2/T3 classification
- Reconciliation marks tasks as "likely_done" (not "done") — operator confirms

## Implementation

| Phase | Description | Gap | Status |
|-------|------------|-----|--------|
| P1 | Brief pagination + summary mode default | G5 | Pending |
| P2 | Auto-reset task.json on main + allow-all when absent | G1 | Pending |
| P3 | MCP session identity auto-registration | G2 | Pending |
| P4 | Workplan reconciliation with git evidence | G3 | Pending |
| P5 | File-overlap detection at dispatch | G4 | Pending |
| P6 | Auto-cleanup worktrees on task completion | G6 | Pending |
| P7 | `hex worktree merge` command | G7 | Pending |

## References

- ADR-004: Git Worktrees for Parallel Agent Isolation
- ADR-048: Claude Code Session Agent Registration
- ADR-2603241700: Worktree Enforcement in Agent Hooks
- ADR-2604051700: Enforce Workplan Gates
- ADR-2604131500: AIOS Developer Experience
- Dog-fooding session 2026-04-13: wp-tiered-inference-routing execution

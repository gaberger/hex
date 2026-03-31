# ADR-2603231700: Worktree Enforcement in Agent Hooks

**Status:** Implemented
**Date:** 2026-03-23
**Drivers:** Background agents bypass worktree isolation (ADR-004) because `pre-agent` hook only validates HEXFLO_TASK presence, not worktree assignment. This was observed during the OpenRouter integration (ADR-2603231600) where 9 agents edited files directly on `main` instead of isolated worktrees.
**Relates to:** ADR-004 (Swarm Worktrees), ADR-050 (Hook-Enforced Lifecycle), ADR-2603221939 (Mandatory Swarm Tracking), ADR-061 (Workplan Lifecycle)

## Context

hex's feature development pipeline (CLAUDE.md Phase 3: WORKTREES) requires each agent to work in an isolated git worktree scoped to its adapter boundary. This prevents:
- Merge conflicts when parallel agents edit overlapping files
- Cross-adapter coupling (agent touches files outside its assigned boundary)
- Unrecoverable state when one agent's changes need to be reverted

**ADR-004** defined the worktree model. **ADR-050** defined the hook pipeline. **ADR-2603221939** enforced `HEXFLO_TASK:` in prompts. But none of them connected the dots: **no hook validates that the agent actually runs in its task's assigned worktree**.

### Current `pre-agent` Hook (line 642 of hook.rs)

```
1. Parse TOOL_INPUT for prompt, subagent_type, run_in_background
2. Exempt read-only agents (Explore, Plan)
3. Check workplan exists → block if mandatory mode + no workplan
4. Check HEXFLO_TASK:{uuid} in prompt → block if background + no task
5. Propagate workplan context
```

**Missing steps:**
- Resolve task UUID → look up `worktree_branch` from workplan
- Check if worktree exists → auto-create if not
- **Inject `cwd` or `isolation: "worktree"` into the agent call** so it runs in the worktree, not on `main`
- Validate file edits stay within the task's adapter boundary

### What Goes Wrong Without Enforcement

When 9 agents run on `main` simultaneously:
1. **No rollback granularity** — can't revert one agent's work without cherry-picking
2. **Silent conflicts** — two agents editing the same file produces last-write-wins, not merge conflicts
3. **No boundary enforcement** — agent assigned to `adapters/secondary/` can freely edit `adapters/primary/`
4. **Merge order bypassed** — changes land on `main` in execution order, not dependency order (domain → ports → adapters)

## Decision

### 1. Extend `pre-agent` Hook with Worktree Resolution

When a background agent has `HEXFLO_TASK:{uuid}`, the hook will:

```
a. Load active workplan from session state
b. Find the task's step in the workplan JSON by matching task title or step ID
c. Read `worktree_branch` from that step
d. Check if worktree exists (git worktree list)
e. If not: auto-create it (git worktree add <path> -b <branch>)
f. Output WORKTREE_PATH to stdout so the orchestrator can set cwd
```

### 2. Add `worktree_path` to SessionState

Extend the session state file with per-task worktree tracking:

```rust
struct SessionState {
    // ... existing fields ...
    /// Active worktree path for current task (if any)
    worktree_path: Option<String>,
    /// Allowed file paths for boundary enforcement
    allowed_paths: Vec<String>,
}
```

### 3. Enforce Adapter Boundary in `pre-edit` Hook

When a workplan is active and the current task has a defined layer/adapter, the `pre-edit` hook will validate that the file being edited falls within the task's boundary:

```
Task layer: "adapters/secondary", adapter: "openai-compat"
  → Allow: hex-agent/src/adapters/secondary/openai_compat.rs ✓
  → Block: hex-agent/src/adapters/primary/cli.rs ✗
  → Allow: tests/ (always allowed)
  → Allow: docs/ (always allowed)
```

Boundary rules:
- `domain` → only `domain/`, `value-objects/`, `entities/`
- `ports` → only `ports/`
- `adapters/primary/{name}` → only files matching the adapter name
- `adapters/secondary/{name}` → only files matching the adapter name
- `usecases` → `usecases/`, `composition-root`
- `integration` → `tests/`, `docs/`
- Files in `docs/`, `tests/`, `config/` are always allowed (cross-cutting)

Mode: **advisory** by default (warn but allow), **mandatory** when `.hex/project.json` has `"boundary_enforcement": "mandatory"`.

### 4. Enforce Dependency Order in `pre-agent` Hook

Before allowing a task to start, validate its dependencies are complete:

```
a. Load workplan steps
b. Find current step's dependencies[]
c. For each dependency, check HexFlo task status
d. If any dependency is not "completed": block with message
```

This prevents tier-2 agents from spawning while tier-1 tasks are still running.

### 5. Auto-Cleanup Worktrees After Task Completion

In the `subagent-stop` hook, after marking a task complete:

```
a. If all tasks in the current tier are complete:
   → Run feature-workflow.sh merge for completed tier
b. If all workplan tasks are complete:
   → Run feature-workflow.sh cleanup
   → Update workplan status to "done"
```

### 6. Worktree Branch Naming Validation

The `pre-agent` hook validates that `worktree_branch` follows the convention:
```
feat/{feature-name}/{layer-or-adapter}
```

Reject arbitrary branch names that would break `feature-workflow.sh merge`.

### 7. Agent `isolation: "worktree"` Integration

When Claude Code's Agent tool supports `isolation: "worktree"`, the orchestrator should use it instead of manual cwd management. Until then, the hook outputs `HEXFLO_WORKTREE_PATH` and the orchestrating agent includes it in the spawned agent's prompt as a working directory instruction.

## Consequences

### Positive
- **True isolation** — each agent works in its own worktree, no conflicts possible
- **Rollback granularity** — revert one agent's work by deleting its worktree branch
- **Boundary enforcement** — agents can only edit files within their assigned adapter
- **Dependency ordering** — tier-2 can't start until tier-1 merges
- **Automatic cleanup** — worktrees removed after successful merge

### Negative
- **Disk usage** — each worktree is a full working copy (~100-300MB per worktree)
- **Merge complexity** — dependency-ordered merge requires all lower tiers to pass tests
- **Latency** — worktree creation adds ~2-5s per agent spawn
- **Fail-open risk** — if workplan JSON is malformed, hook falls back to allowing main

### Mitigations
- **Disk**: Max 8 concurrent worktrees (already enforced in feature-workflow.sh)
- **Merge**: Automated via `feature-workflow.sh merge` with rebase-first strategy
- **Latency**: Worktree creation is one-time per task; subsequent agent resumes reuse it
- **Fail-open**: Log warnings when falling back; `hex agent audit` flags main-branch edits

## Implementation

| Phase | Description | Status |
|-------|------------|--------|
| P1 | Add `worktree_path`, `allowed_paths` to SessionState | Pending |
| P2 | Worktree resolution in `pre-agent`: lookup task → workplan step → worktree_branch | Pending |
| P3 | Auto-create worktree if missing (git worktree add) | Pending |
| P4 | Boundary enforcement in `pre-edit`: validate file path against task's layer/adapter | Pending |
| P5 | Dependency order enforcement in `pre-agent`: check prerequisite tasks completed | Pending |
| P6 | Worktree branch naming validation | Pending |
| P7 | Auto-merge + cleanup in `subagent-stop` on tier completion | Pending |
| P8 | `hex agent audit` — flag agents that edited main without worktree | Pending |

## References

- ADR-004: Git Worktrees for Parallel Agent Isolation
- ADR-050: Hook-Enforced Agent Lifecycle Pipeline
- ADR-054: ADR Compliance Enforcement

## Implementation Notes

Implemented in:
- `hex-cli/src/commands/hook.rs` — `SubagentStart` handler (~lines 440-690), `check_tier_gate` (~line 635), `ensure_worktree_exists` (~line 477)
- `hex-cli/src/commands/agent_audit.rs` — agent audit trail flagging main-branch edits without worktree
- ADR-2603221939: Mandatory Swarm Tracking for Background Agents
- ADR-061: Workplan Lifecycle Management
- Incident: OpenRouter integration (ADR-2603231600) — 9 agents on main, no worktree isolation

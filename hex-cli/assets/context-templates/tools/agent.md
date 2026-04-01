Launch a new agent to handle complex, multi-step tasks autonomously. Each agent type has specific capabilities and tools available to it.

When the agent is done, it returns a single message — the result is not visible to the user; send a text summary.

You can run agents in the background using the run_in_background parameter. When running in the background, you will be notified when it completes — do NOT poll or sleep.

Background agents that edit files MUST use mode=bypassPermissions.

## hex-specific rules

### HexFlo task tracking (ADR-048)

Before spawning any agent for a HexFlo swarm task:
1. Create a swarm and task via `mcp__hex__hex_hexflo_task_create` (or CLI)
2. Include `HEXFLO_TASK:{task_id}` at the START of the agent prompt
3. Hooks auto-update task status: `SubagentStart` → `in_progress`, `SubagentStop` → `completed`

```
# Correct agent prompt structure
HEXFLO_TASK:88bb424c-591a-482e-ac4f-55969549b7cf
Implement the secondary adapter for...
```

### Worktree isolation (ADR-004)

Background agents that write code MUST operate in a git worktree — never on main branch.
Worktrees are named `feat/<feature-name>/<layer-or-adapter>`.

Use the `hex-worktree` skill or `./scripts/feature-workflow.sh setup` to create worktrees before spawning agents.

### Always commit explicitly

Worktree agents do NOT auto-commit. Every agent prompt that writes code MUST end with:
```
When done: git add <specific-files> && git commit -m "feat(...): ..."
```

### Agent roles

| subagent_type | Use for |
|---|---|
| `hex-coder` | Implementing within a single adapter boundary (TDD) |
| `hex-planner` | Decomposing requirements into workplan steps |
| `hex-reviewer` | Reviewing architecture compliance and code quality |
| `general-purpose` | Research, exploration, multi-step analysis |

### Mode selection

| Scenario | mode |
|---|---|
| Background agent writing files | `bypassPermissions` (REQUIRED) |
| Foreground interactive agent | `default` |
| Agent in worktree isolation | `bypassPermissions` |

**Never use `acceptEdits` for background agents** — it silently denies all file writes.

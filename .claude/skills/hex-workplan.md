---
name: hex-workplan
description: Create, validate, and execute a hex workplan. Enforces ADR-before-code, specs-first, HexFlo swarm tracking, worktree isolation, and Path B (Claude Code) dispatch protocol.
trigger: /hex-workplan
---

# hex Workplan Execution

Workplans decompose a feature into dependency-ordered phases, each bounded by a hexagonal
architecture layer. Execution creates HexFlo swarm tasks, isolates agents in git worktrees,
and enforces phase gates before advancing.

**Required pipeline order** (ADR-050, ADR-2604051700):
```
ADR (docs/adrs/) → Specs (docs/specs/) → Workplan (docs/workplans/) → Execute
```
Never skip steps. `hex_plan_execute` validates this at pre-flight and will warn if violated.

---

## Step 1: Verify prerequisites

```bash
hex adr list          # confirm referenced ADR exists and is accepted
hex plan list         # check for existing workplans on this feature
```

---

## Step 2: Workplan schema

Workplans live in `docs/workplans/wp-<feature>.json`. Required fields:

```json
{
  "id": "wp-<feature>",
  "feature": "Human-readable name",
  "description": "What this delivers and why",
  "adr": "ADR-YYMMDDHHMM",
  "specs": "docs/specs/<feature>.json",
  "status": "planned",
  "created_at": "2026-04-08T00:00:00Z",
  "created_by": "planner",
  "phases": [
    {
      "id": "P0",
      "name": "Domain & Ports",
      "tier": 0,
      "gate": { "type": "typecheck", "command": "cargo check --workspace", "blocking": true },
      "tasks": [
        {
          "id": "P0.1",
          "name": "Define port trait",
          "layer": "ports",
          "description": "Implementation details and acceptance criteria",
          "deps": [],
          "files": ["hex-core/src/ports/example.rs"],
          "agent": "hex-coder"
        }
      ]
    }
  ]
}
```

Key rules:
- Phase `id` must match `^P\d+$` (P0, P1, P2…)
- Task `id` must match `^P\d+\.\d+$` (P0.1, P1.2…)
- Task `layer` must be one of: `domain`, `ports`, `primary`, `secondary`
- Task `agent` must be one of: `hex-coder`, `planner`, `integrator`, `reviewer`
- `gate` is required between phases — use `cargo check` or `bun test` as appropriate

---

## Step 3: Execute

```
mcp__hex__hex_plan_execute(file: "docs/workplans/wp-<feature>.json")
```

Or via CLI: `hex plan execute docs/workplans/wp-<feature>.json`

---

## Path B: Claude Code session dispatch (CLAUDECODE=1)

When hex-nexus detects it is running inside a Claude Code session (`CLAUDECODE=1`), it uses
**Path B** instead of spawning hex-agent processes directly:

1. For each task, nexus creates an **inference task** in SpacetimeDB and sends a **priority-2
   inbox notification** (`type: inference-queue`) to the active Claude Code agent.
2. Nexus then **polls** SpacetimeDB waiting for the task to be marked `Completed` or `Failed`.
3. The outer Claude Code agent (you) **must handle these notifications** — the pipeline stalls
   until you do.

### How to handle an `inference-queue` inbox notification

When `hex hook route` surfaces an `inference-queue` priority-2 notification:

1. **Acknowledge** the notification:
   ```
   mcp__hex__hex_inbox_ack(id: "<notification_id>")
   ```

2. **Read the queued task payload** from the notification body — it contains:
   - `queue_id` — the inference task ID in SpacetimeDB
   - `task_id` — the workplan task ID (e.g. "P1.2")
   - `workplan_id` — the workplan being executed
   - The full task **prompt** (already built by the executor, includes HEXFLO_TASK token)

3. **Spawn an Agent tool** with the task prompt:
   ```
   Agent({
     subagent_type: "coder",           // or "general-purpose"
     mode: "bypassPermissions",        // REQUIRED for background file writes
     run_in_background: true,
     prompt: "<full prompt from payload>"
   })
   ```
   The prompt already contains `HEXFLO_TASK:{id}`, role preamble, files list, and the
   required `git commit` instruction. Do not modify it.

4. Wait for the agent to complete, then **mark the inference task done**:
   ```bash
   hex workplan task-complete <queue_id>
   ```
   This unblocks the nexus polling loop and advances to the next task.

5. If the agent fails, mark it failed:
   ```bash
   hex workplan task-fail <queue_id> "reason"
   ```

### Common mistakes that break Path B

| Mistake | Consequence |
|---------|-------------|
| Spawning agents directly without handling inbox | Nexus polls forever (30-min timeout) |
| Using `mode: acceptEdits` for background agents | File writes silently denied |
| Not calling `task-complete` after agent finishes | Next task never starts |
| Writing workplan to wrong directory | `hex_plan_execute` reads wrong path |
| Referencing ADR that doesn't exist | Pre-flight warning; pipeline continues but violates ADR-before-code |

---

## Checking execution status

```
mcp__hex__hex_plan_status        # current phase + progress
mcp__hex__hex_plan_report(id: "<execution_id>")  # full report with gate results
```

---

## What the executor does automatically

- Creates a HexFlo swarm for the workplan execution
- Creates one HexFlo task per workplan task (for tracking)
- Prepends `HEXFLO_TASK:{id}` to every agent prompt
- Injects role preamble based on the `agent` field
- Appends a required git commit instruction to every prompt
- Runs phase gates between phases (`cargo check`, `bun test`, etc.)
- Stores task outcomes in HexFlo memory ledger

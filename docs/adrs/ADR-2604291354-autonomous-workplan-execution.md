# ADR-2604291354 — Autonomous Workplan Execution

**Status:** Accepted (2026-05-05)
**Date:** 2026-04-29
**Supersedes:** —
**Related:** ADR-2604141400 (brain queue swarm-lease), ADR-2604131630 (code-first execution), ADR-2604110227 (task tier routing)

## Context

`hex sched enqueue workplan <path>` creates a brain-task, daemon picks it up, sends to nexus, nexus dispatches via "Path B" (workplan sent to nexus for execution, Claude handles inference). But nothing actually executes the tasks.

**Observed behavior (wp-memory-health-swarm, task 325e0bba):**
```
⬡ Execution started: 4df3eb1c-c44b-4b93-828d-fa874f245b83
  ♡ [30s] status: running
  ♡ [60s] status: running
  ...
  ♡ [571s] status: running

--- stderr ---
Workplan execution timed out after 600s

--- guard ---
pre_head=f3b6de65  post_head=f3b6de65
no git evidence: HEAD unchanged
```

The daemon polls for 10 minutes, sees no git commits, marks the task `failed`. `hex monitor` shows nothing because the task completed (failed) before user checked.

**Root cause:** "Path B" assumes an active Claude Code session is listening and will execute the workplan. But when the user is not in an active Claude session (or session is idle), no agent picks up the work. The enqueue → dispatch → wait loop is a **coordination gap** — the daemon expects execution, but no executor exists.

**Why this is critical:**
- User expectation: *"hex should enqueue work to finish"* → work gets enqueued, but doesn't finish
- Memory-health swarm (just drafted) will fail the same way
- Idle-research swarm (ADR-2604151200) will fail the same way
- Any autonomous improvement loop requires this: discover → enqueue → **execute** → learn

The AIOS can't "get smarter every run" if it can only discover problems but never autonomously fix them.

## Decision

Implement **autonomous workplan execution** — when the daemon leases a workplan task and no active Claude session exists (or session is idle), the daemon spawns a **background agent** to execute the workplan.

### Architecture

```
User enqueues workplan
  ↓
Daemon leases task
  ↓
Check: Active Claude session? ───YES──→ Path B (dispatch to session)
  ↓ NO                                    [existing behavior]
  ↓
Path C: Spawn background agent
  ├─ Fork process: hex-agent --workplan <id> --background
  ├─ hex-agent reads workplan JSON
  ├─ For each pending task:
  │   ├─ Determine tier (T1/T2/T2.5) from strategy_hint
  │   ├─ Route to appropriate inference backend
  │   ├─ Execute with compile gate (cargo check / tsc --noEmit)
  │   ├─ Commit on success, roll back on failure
  │   └─ Update task status in workplan JSON
  ├─ Write result summary to stdout
  └─ Exit with code 0 (success) or 1 (failure)
  ↓
Daemon reads exit code + stdout
  ↓
Update brain-task: status=completed/failed, result=<summary>
```

### Implementation phases

**P1: hex-agent workplan executor**
- New binary entry point: `hex-agent --workplan <id> --background`
- Reads workplan JSON from disk (or memory if id-based)
- Iterates tasks with `status=pending`, skips `done`/`in_progress`
- For each task:
  - Parse `strategy_hint` → tier (scaffold/transform/script → T1, codegen → T2, inference → T2.5)
  - Call inference backend with task prompt + evidence commands
  - Run compile gate (if Rust/TS project)
  - Write files, commit with task ID in message
  - Update workplan JSON task status → `done` on success, `failed` on error
- Write summary to stdout: `{"completed": N, "failed": N, "duration_s": T}`
- Exit 0 if all tasks succeeded, exit 1 if any failed

**P2: Daemon integration**
- In `execute_brain_task` (workplan kind), check for active Claude session:
  - `CLAUDE_SESSION_ID` set → Path B (existing dispatch to nexus)
  - `CLAUDE_SESSION_ID` unset → Path C (spawn hex-agent)
- Path C:
  ```rust
  let output = Command::new("hex-agent")
      .args(["--workplan", &task_id, "--background"])
      .output()?;
  let summary = String::from_utf8_lossy(&output.stdout);
  if output.status.success() {
      mark_completed(&task_id, &summary).await?;
  } else {
      mark_failed(&task_id, &summary).await?;
  }
  ```
- Log to daemon output: `⬡ spawned autonomous agent for workplan <id>`

**P3: Inference backend selection**
- hex-agent uses `InferenceRouter` (existing, from ADR-2604120202)
- T1 → qwen3:4b (scaffold/transform/script)
- T2 → qwen2.5-coder:32b (codegen)
- T2.5 → devstral-small-2:24b (complex reasoning)
- T3 → Claude (frontier) — requires CLAUDE_SESSION_ID, falls back to T2.5 if unset

**P4: Compile gate + rollback**
- After writing files, run compile check:
  - Rust: `cargo check --workspace`
  - TypeScript: `tsc --noEmit`
  - Other: skip gate (trust inference output)
- If check fails:
  - Rollback: `git reset --hard HEAD`
  - Mark task `failed` with compiler error in result
  - Stop workplan execution (don't cascade failures)
- If check passes:
  - Commit: `git commit -m "feat(<task.id>): <task.title>"`
  - Continue to next task

**P5: Observability**
- `hex monitor` shows autonomous agent activity:
  - Queue status includes "background agent: <workplan-id>"
  - Recent activity shows autonomous tasks with agent icon (🤖)
- Dashboard panel: "Autonomous Execution" showing active agents, success rate, avg duration
- Logs: `~/.hex/agent-<workplan-id>.log` for debugging

### Configuration

`.hex/project.json`:
```json
{
  "autonomous": {
    "enabled": true,
    "max_concurrent_agents": 3,
    "compile_gate": true,
    "rollback_on_failure": true,
    "inference": {
      "tier_fallback": "T2.5"  // when T3 unavailable
    }
  }
}
```

### Opt-outs

- `HEX_AUTONOMOUS=0` env var disables Path C globally
- `.hex/project.json` → `autonomous.enabled: false`
- Per-workplan: `"autonomous": false` in workplan JSON

## Consequences

**Positive:**
- Workplans enqueued via daemon actually execute
- Memory-health swarm runs autonomously every hour
- Idle-research swarm runs autonomously when idle
- User can enqueue work and walk away — system completes it
- Closes the self-learning loop: discover → enqueue → **execute** → learn

**Negative / risks:**
- Background agent could produce bad code
  - **Mitigation:** compile gate + rollback prevents broken commits
  - **Mitigation:** T1/T2 local models (cheap, fast feedback) for most work
  - **Mitigation:** human review via git log — all commits tagged with task ID
- Multiple concurrent agents could conflict (edit same file)
  - **Mitigation:** `max_concurrent_agents` cap (default 3)
  - **Mitigation:** workplan task `files` array declares intent — daemon can serialize overlapping work
- Agent could run indefinitely (hung inference)
  - **Mitigation:** workplan `timeout_s` honored — daemon kills agent after timeout
  - **Mitigation:** per-task timeout (default 300s) — agent aborts stuck inference

**Open questions:**
- Should agents share a memory context (learn from each other)?
  - **Decision:** yes — agents write findings to memory via `hex memory store`, categorizer reads in next health-check
- Should failed autonomous tasks auto-retry?
  - **Decision:** yes, via memory-health swarm categorization (transient → retry, blocker → escalate)
- Should agents commit after each task or batch at end?
  - **Decision:** commit after each task — better git evidence, easier rollback

## Non-goals

- **Not replacing interactive sessions.** Path B (dispatch to Claude session) remains for user-driven work.
- **Not general-purpose agent spawning.** Scoped to workplan execution only.
- **Not distributed execution.** Single-host only; remote agents are follow-up.

## Implementation

See `wp-autonomous-workplan-execution.json`.

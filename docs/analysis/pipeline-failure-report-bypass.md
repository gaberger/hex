# Pipeline Failure Report: Structured Workplan Tasks Bypassed

**Date**: 2026-03-25
**Swarm under investigation**: `create-a-cli-temperature-converter-that` (and any `hex dev` run)
**Symptom**: P0.1–P4.1 HexFlo tasks created but never executed; pipeline runs `hex-coder: CodeGenerated` directly instead.

---

## 1. The Two Independent Task-Creation Systems

There are **two completely separate task-creation flows** that have no knowledge of each other:

### Path A — SwarmPhase (structural P* tasks)

**File**: `hex-cli/src/pipeline/swarm_phase.rs` and `hex-cli/src/tui/mod.rs:655–695`

`SwarmPhase::execute()` iterates `workplan.steps` and calls `runner.task_create(swarm_id, title)` for each step, producing HexFlo tasks with titles like `"P0.1: domain layer"`. These task IDs are stored in `self.task_id_map: HashMap<String,String>` (step_id → hexflo_task_id) on the TUI state.

**This phase completes and the task IDs sit in `self.task_id_map`.  Nothing ever uses them to drive execution.**

### Path B — Supervisor + CodePhase (objective-driven execution)

**File**: `hex-cli/src/tui/mod.rs:992–1004`

```rust
let supervisor = Supervisor::new(&output_dir, language)
    .with_tracking(
        self.session.swarm_id.clone(),   // ← swarm_id only
        self.session.agent_id.clone(),
    )
    .with_session(shared_session.clone());

let supervisor_fut = supervisor.run(&workplan_data, &adr_content);
```

`Supervisor::with_tracking()` accepts `(swarm_id, agent_id)` — it does **not** accept the `task_id_map`. There is no parameter for passing the pre-created P* task IDs into the supervisor.

---

## 2. How the Supervisor Actually Executes

**File**: `hex-cli/src/pipeline/supervisor.rs:811–961` (`Supervisor::run`)

The supervisor groups workplan steps by tier (`0..=max_tier`) and calls `run_tier()` for each. It does **not** iterate over the pre-created HexFlo task IDs.

**File**: `hex-cli/src/pipeline/supervisor.rs:967–1100` (`run_tier`)

`run_tier()` runs an objective evaluation loop (max 5 iterations):
1. Call `evaluate_all()` — checks `CodeGenerated`, `CodeCompiles`, `TestsExist`, `TestsPass`, `ReviewPasses`, `ArchitectureGradeA`.
2. Find the first unmet objective.
3. Look up `agent_for_objective(obj, has_prior)`.
4. For `CodeGenerated` (unmet on iteration 1 because no `src/` files exist yet): `agent_for_objective` returns `"hex-coder"`.
5. Call `execute_agent_tracked("hex-coder", ...)` → `dispatch_agent("hex-coder", ...)`.

**File**: `hex-cli/src/pipeline/supervisor.rs:1362–1484` (`dispatch_agent`)

The `hex-coder` branch calls `CodePhase::execute_step()` directly for each `workplan_steps` entry. It **creates its own fresh HexFlo tracking task** at `supervisor.rs:1113–1131` via `create_tracking_task(role, &state.objective, iteration)` with the title `"hex-coder: CodeGenerated [iteration 1]"`.

This is the task that appears in the dashboard. It is a **new task created by the supervisor's tracking layer** — not one of the P* tasks from SwarmPhase.

---

## 3. The Exact Disconnect

`SwarmPhase` creates tasks `P0.1`, `P1.1`, etc. and returns them in `SwarmPhaseResult::task_ids`. The TUI stores them in `self.task_id_map`. Then:

```
tui/mod.rs:689:  self.task_id_map = r.task_ids.iter().cloned().collect();
...
tui/mod.rs:992:  let supervisor = Supervisor::new(...)
                     .with_tracking(swarm_id, agent_id);  // ← task_id_map NOT passed
tui/mod.rs:1004: supervisor.run(&workplan_data, &adr_content);
```

`Supervisor::with_tracking()` signature (`supervisor.rs:154–162`):
```rust
pub fn with_tracking(
    mut self,
    swarm_id: impl Into<Option<String>>,
    agent_id: impl Into<Option<String>>,
) -> Self
```

There is no `task_id_map` parameter. The supervisor has no way to know the P* task IDs exist.

When the supervisor **does** need to track tasks, it calls `create_tracking_task()` which posts to `hex task create` under the same swarm, producing **new** tasks named `"hex-coder: CodeGenerated [iteration N]"`. These are structurally parallel to the P* tasks rather than being the same tasks.

The P* tasks remain forever in `pending` status — they were never assigned, never started, never completed.

---

## 4. Is This a Bug or a Design Choice?

**This is a bug — a design seam that was never closed.**

The original design intent (visible from `code_phase.rs:740–824`, `execute_all_tracked`) was that the P* task IDs would be passed into the code execution layer. `CodePhase::execute_all_tracked()` accepts a full `task_id_map: &HashMap<String, String>` and properly marks each P* task as `in_progress` then `completed` as code is generated for each step.

However, the TUI was later refactored to go through `Supervisor::run()` (the objective-loop architecture) instead of `CodePhase::execute_all_tracked()`. The supervisor has its own internal task creation (`create_tracking_task`) but was never wired to receive or use the pre-created P* task IDs from `SwarmPhase`.

The result: `CodePhase::execute_all_tracked()` (which would correctly drive P* tasks) is **never called from the main pipeline path**. The supervisor's objective loop drives execution instead, creating its own shadow tasks.

---

## 5. Root Cause Summary

| Component | What it does | What it should do |
|-----------|-------------|-------------------|
| `SwarmPhase::execute()` | Creates P0.1–P4.1 HexFlo tasks, stores IDs in `task_id_map` | Same — correct |
| `tui/mod.rs:992` | Creates `Supervisor` with `.with_tracking(swarm_id, agent_id)` | Should also call `.with_task_map(self.task_id_map.clone())` |
| `Supervisor::with_tracking()` | Accepts only `swarm_id` + `agent_id` | Should accept `task_id_map: HashMap<String,String>` |
| `Supervisor::create_tracking_task()` | Creates **new** tasks for each agent invocation | Should instead mark the **existing** P* task as `in_progress`/`completed` |
| `Supervisor::run()` → `run_tier()` | Drives execution via objective loop | Same — correct architecture |

**The fix requires**: threading `task_id_map` from `SwarmPhaseResult` through `Supervisor::with_tracking()` into `execute_agent_tracked()`/`create_tracking_task()`, so that when the supervisor starts working on a step that matches a P* task ID, it marks that existing task rather than creating a new one.

---

## 6. Affected Files

- `/Volumes/ExtendedStorage/PARA/01-Projects/hex-intf/hex-cli/src/tui/mod.rs` — call site that drops `task_id_map` (line 992–1004)
- `/Volumes/ExtendedStorage/PARA/01-Projects/hex-intf/hex-cli/src/pipeline/supervisor.rs` — `with_tracking()` missing `task_id_map` param; `create_tracking_task()` creates shadow tasks instead of reusing P* tasks
- `/Volumes/ExtendedStorage/PARA/01-Projects/hex-intf/hex-cli/src/pipeline/swarm_phase.rs` — produces `task_ids` that go unused downstream
- `/Volumes/ExtendedStorage/PARA/01-Projects/hex-intf/hex-cli/src/pipeline/code_phase.rs` — `execute_all_tracked()` is the correct tracked API but is never called from the live pipeline path

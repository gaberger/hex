# ADR-2603291900: Docker Worker First-Class Execution

**Status:** Proposed
**Date:** 2026-03-29
**Drivers:** Docker sandbox workers are spawned per pipeline run but never delegated real tasks — all inference and file generation runs inline in the supervisor process. Workers idle until the pipeline completes.
**Supersedes:** —

<!-- ID format: YYMMDDHHMM — use your local time. Example: 2603221500 = 2026-03-22 15:00 -->

## Context

The hex dev pipeline spawns Docker sandbox workers for each agent role (`hex-coder`, `hex-reviewer`, `hex-tester`, `hex-analyzer`, `hex-fixer`, `hex-ux`) at the start of every run. These workers register with nexus, send heartbeats, and poll for tasks — but are never assigned real work.

Worker delegation is explicitly disabled in `supervisor.rs`:
```rust
#[allow(clippy::overly_complex_bool_expr)]
let agent_result = if false && self.has_worker_for_role(role) {
```

The comment reads: *"Worker delegation is currently disabled — task_assign/task_status plumbing is not fully wired end-to-end."*

The `hex-coder` worker implementation in `agent.rs` is a stub:
- Creates a `CodePhase` but never calls it
- Returns `"hex-coder: executed task '{title}'"` with `files: []`
- Does not perform inference, generate code, or write files to disk

Other worker roles (`hex-reviewer`, `hex-tester`, `hex-analyzer`) do have real agent implementations but are also unreachable due to the `if false` guard.

**Forces at play:**
- Docker sandbox isolation is a security and reproducibility requirement — generated code should run in a contained environment
- Workers need the full `WorkplanStep` context (not just title) to do real code generation
- Workers need `output_dir` (the Docker-mounted project path) to write files
- Results must be surfaced back to the supervisor so it can evaluate objectives (compile, test, review)
- The supervisor's inline TDD loop (compile → test → fix) must be replicated or delegated

**What the worker needs to become first-class:**
1. Full `WorkplanStep` JSON stored in task metadata (not just title)
2. `output_dir` passed as task metadata or environment variable
3. `hex-coder` worker runs the full `CodePhase` + TDD loop inline
4. Generated file paths + content written to hexflo memory so supervisor can evaluate objectives
5. Test results, compile status, and review verdicts surfaced back via task result
6. Supervisor's `if false &&` guard removed

## Decision

We will make Docker workers first-class execution environments for all pipeline roles:

1. **Task metadata enrichment**: When the supervisor creates a HexFlo task for a workplan step, it will store the full `WorkplanStep` JSON and `output_dir` in the task's `description` field (JSON-encoded) so workers have complete context.

2. **`hex-coder` worker implementation**: Replace the stub with a real implementation that:
   - Deserializes `WorkplanStep` from task metadata
   - Calls `CodePhase::execute_step()` with the step and inferred language
   - Writes generated files to the Docker-mounted `output_dir`
   - Runs compile + test checks (`go build`/`cargo check`/`tsc`)
   - Stores file paths + compile/test results in hexflo memory under `{task_id}:result`
   - Returns a structured result string the supervisor can parse for objective evaluation

3. **Supervisor objective evaluation from worker results**: After a worker task completes, the supervisor reads `{task_id}:result` from hexflo memory to update `ObjectiveState` (CodeGenerated, CodeCompiles, TestsPass).

4. **Enable delegation**: Remove the `if false &&` guard. The supervisor will delegate to workers when `has_worker_for_role(role)` is true, falling back to inline when no worker is available.

5. **Output dir via env var**: Workers are launched with `HEX_OUTPUT_DIR` set to the absolute output directory, providing a reliable fallback if task metadata parsing fails.

## Consequences

**Positive:**
- Generated code runs in isolated Docker containers — no risk of generated code affecting the host
- Parallel execution becomes possible: multiple coder workers can run different tiers simultaneously
- Worker crashes are isolated from the supervisor process
- Reproducible builds: Docker image pins all tool versions (Go, Node, Rust)

**Negative:**
- File I/O crosses the Docker boundary — generated files must be on a mounted volume
- Debugging is harder — logs are in the container, not the supervisor terminal
- Task metadata size is bounded — very large workplan steps may need chunking

**Mitigations:**
- Docker volume mount at `abs_output` already established in `spawn_workers`
- Worker logs forwarded to nexus via stdout capture (already partially wired)
- Workplan step sanitization keeps steps small (single file per step for Go/Rust)

## Implementation

| Phase | Description | Status |
|-------|------------|--------|
| P0 | Enrich task metadata: store `WorkplanStep` JSON + `output_dir` when creating tracking tasks | Pending |
| P1 | Implement real `hex-coder` worker: call `CodePhase`, write files, run compile+test, store result in hexflo memory | Pending |
| P2 | Supervisor reads worker result from hexflo memory to update `ObjectiveState` | Pending |
| P3 | Set `HEX_OUTPUT_DIR` env var when spawning workers | Pending |
| P4 | Remove `if false &&` guard and enable worker delegation | Pending |
| P5 | Integration test: run full pipeline with workers, verify files land at correct path inside Docker mount | Pending |

## References

- `hex-cli/src/pipeline/supervisor.rs` — `execute_agent_tracked` (delegation guard), `spawn_workers` (Docker launch)
- `hex-cli/src/commands/agent.rs` — `execute_worker_task` (stub implementation)
- ADR-2603240130 — Declarative Swarm Behavior from YAML
- ADR-2603241230 — Agent Coordination (worker registration + heartbeat)

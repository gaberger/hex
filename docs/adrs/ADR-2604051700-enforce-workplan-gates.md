# ADR-2604051700: Enforce Workplan Gates — Instructions to Code

**Status:** Accepted
**Date:** 2026-04-05
**Drivers:** ADR-2604050900 session demonstrated that workplan gates exist as JSON declarations but are not enforced as blocking prerequisites. Missing behavioral spec, undocumented gate execution, and unscanned deletion blast radius all passed through unchecked. hex's specs-first pipeline is only specs-first if something enforces the ordering.

## Context

hex workplans define three types of safeguards:

1. **Spec references** — `"specs": "docs/specs/foo.json"` — the behavioral spec that must exist before code is written
2. **Phase gates** — `"gate": {"command": "cargo check ...", "blocking": true}` — build/test commands between phases
3. **Deletion tasks** — tasks whose description involves removing modules, files, or crate members

During the ADR-2604050900 session, all three failed silently:
- The spec file didn't exist but the workplan executed anyway
- Phase gates were declared but their execution was not auditable
- 12 module deletions were not scanned for cross-workspace consumers, causing 21 compilation errors and 7 rounds of reactive fixing

The workplan executor (`hex-nexus/src/orchestration/workplan_executor.rs`) already has `run_gate()` infrastructure (line 942) that executes shell commands and checks exit codes. The gap is not capability — it's enforcement scope.

## Decision

Add three blocking pre-flight and inter-phase gates to the workplan executor:

### Gate 1: Spec-File-Exists (Pre-Flight)

Before `run_phases()` begins, if `workplan.specs` is non-empty, verify the file exists on disk. Fail with a clear error if missing. This enforces the specs-first pipeline at the machine level.

### Gate 2: Consumer-Scan (Pre-Deletion)

When a task's `files` array contains paths being deleted (detected by task description containing "delete" or "remove"), automatically run `grep -r "{basename}" --include="*.rs" --include="*.ts"` across the workspace before the task executes. If matches are found outside the deletion set, log them as warnings. If the phase gate is `blocking: true`, halt execution.

### Gate 3: Worktree Cleanup (Post-Agent)

When the `SubagentStop` hook fires and the agent was running in a worktree (detected by `CLAUDE_WORKTREE` env var or worktree path in session file), automatically run `git worktree remove` for that worktree if the agent completed successfully. This prevents orphaned worktrees from accumulating.

## Consequences

**Positive:**
- Missing specs are caught before any code is generated
- Deletion blast radius is scanned automatically, preventing the "21 errors in hex-agent" class of failure
- Worktrees are cleaned up as agents complete, not left as manual cleanup
- All gate executions are persisted in `ExecutionState.gate_results` for auditability

**Negative:**
- Consumer-scan adds latency (~2-5s per deletion task for workspace-wide grep)
- Strict spec enforcement may block rapid prototyping (mitigated: only enforced when `specs` field is non-empty)

## Implementation

| Change | File | Description |
|--------|------|-------------|
| Spec validation | `workplan_executor.rs` | Add `specs` field to `Workplan` struct; validate file exists before `run_phases()` |
| Consumer scan | `workplan_executor.rs` | Add `run_consumer_scan()` that greps workspace for references to files being deleted |
| Worktree cleanup | `hook-handler.cjs` | Add `git worktree remove` call in `post-task` handler when worktree path exists in session |

## References

- ADR-2604050900: SpacetimeDB Right-Sizing (the session that exposed these gaps)
- ADR-046: Workplan Execution Engine (introduced gate infrastructure)

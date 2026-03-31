# ADR-2603312300: Workplan Live Execution Overlay in `hex plan list`

**Status:** Proposed
**Date:** 2026-03-31
**Drivers:** `hex plan list` reads static JSON `status` fields; live execution progress in SpacetimeDB is never reflected, making the list stale the moment a workplan starts executing.

## Context

`hex plan list` scans `docs/workplans/*.json`, counts tasks with `status: "completed"/"done"`, and displays that as progress. But the workplan executor tracks task completion in SpacetimeDB (`hexflo_memory` table, key `workplan:<execution_id>`). These two tracking layers are never reconciled, so a workplan with 3/28 tasks completed via `hex_plan_execute` still shows `completed: 0` in the list.

### Forces

- JSON files are the source of truth for workplan *definition* (phases, tasks, gates)
- SpacetimeDB execution state is the source of truth for *runtime progress*
- Multiple executions can run the same workplan file (re-runs, partial runs)
- `hex plan list` is a fast read — must not add significant latency

## Decision

Overlay live execution counts onto `hex plan list` output:

1. **Execution index**: when `hex plan list` runs, query SpacetimeDB for all keys matching `workplan:*` via `hexflo_memory_search("workplan:")`
2. **Match by path**: each `ExecutionState` has `workplan_path` (e.g. `docs/workplans/feat-context-engineering.json`). Match against the filename of each listed workplan.
3. **Take latest**: if multiple executions exist for the same workplan, use the one with the most recent `updated_at`.
4. **Overlay counts**: replace the JSON-derived `completed` count with `execution.completed_tasks` and surface `execution.status` (running/completed/failed) as a badge.
5. **Graceful fallback**: if SpacetimeDB is unavailable or no execution found, fall back to JSON file counts (current behavior).

## Implementation

| Phase | Task | Layer |
|---|---|---|
| P1 | Add `hex plan list --live` flag + query execution store | CLI primary |
| P2 | Nexus route `GET /api/workplan/by-path?path=<filename>` to look up execution by workplan filename | nexus routes |
| P3 | Wire overlay into `hex plan list` output (default on, fallback to static) | CLI primary |

## Consequences

**Positive:**
- `hex plan list` shows real-time progress for active executions
- No JSON write-back needed — execution state stays in SpacetimeDB
- Consistent with ADR-032 (SpacetimeDB as single state authority)

**Negative:**
- Adds one REST call per `hex plan list` invocation (mitigated: single batch search, not per-workplan)
- `hex plan list` output differs depending on whether nexus is running

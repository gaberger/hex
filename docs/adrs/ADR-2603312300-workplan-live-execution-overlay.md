# ADR-2603312300: Workplan Live Execution Overlay

**Status:** Accepted
**Date:** 2026-03-31
**Deciders:** hex core team

## Context

`hex plan list` showed static workplan metadata from disk but had no visibility into active execution state. During a running pipeline, users had no way to see which phase was in-flight, what the current step was, or whether the execution was progressing — without switching to the dashboard or tailing logs.

## Decision

`hex plan list` queries the hex-nexus REST API (`GET /api/plans/active`) and overlays live execution state on top of the static workplan list when nexus is available. When nexus is unavailable, the command falls back gracefully to showing static metadata from disk.

The overlay displays:
- Active plan indicator (highlighted row)
- Current phase name and step index
- Execution status (running / paused / failed)
- Elapsed time since phase start

This makes `hex plan list` the canonical terminal view for monitoring live pipeline execution without leaving the CLI.

## Consequences

- **Positive**: Single command gives complete picture — what plans exist + what's actively running
- **Positive**: Graceful degradation — offline / nexus-down scenarios still show static list
- **Positive**: No polling overhead — nexus query is a single HTTP GET per invocation
- **Negative**: Slight latency on `hex plan list` when nexus is reachable (acceptable, sub-100ms)

## Implementation

Complete as of commits `701f69f2` + `c889452b`.

Path B pipeline smoke test passed end-to-end:
- `hex plan list` correctly overlays active execution from nexus
- Static fallback confirmed when nexus unavailable
- Live overlay shows phase name, step index, and status for in-flight plans
- All existing plan list behavior preserved when no active execution

Relevant files:
- `hex-cli/src/commands/project.rs` — plan list command with overlay logic
- `hex-nexus/src/routes/projects.rs` — `/api/plans/active` endpoint

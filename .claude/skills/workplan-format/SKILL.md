---
name: workplan-format
description: Defines the hex workplan JSON format and how workplans guide HexFlo swarm execution. Always loaded when planning features or coordinating swarms.
always_load: true
---

# Workplan Format — Swarm Execution Guide

A workplan is a JSON document in `docs/workplans/` that decomposes a feature into dependency-ordered phases of tasks. Each task maps to one hexagonal adapter boundary. HexFlo swarms execute workplans by creating tasks from the plan and assigning agents to them.

## Canonical Workplan Schema

Use `hex plan schema` to output this as JSON Schema.

```json
{
  "id": "wp-<feature-name>",
  "feature": "Human-readable feature name — used as display title",
  "description": "What this workplan delivers and why",
  "adr": "ADR-NNN",
  "priority": "P0-BLOCKER | high | normal",
  "created_at": "ISO 8601 timestamp",
  "created_by": "planner",
  "supersedes": "wp-older-plan-id (optional)",
  "supersession_reason": "Why the old plan was absorbed (optional)",
  "relates_to": "wp-related-plan-id (optional)",
  "blocks": ["Description of what this workplan blocks (optional)"],

  "phases": [
    {
      "id": "P1",
      "name": "Phase name — what this phase delivers",
      "tier": 0,
      "description": "Detailed description of the phase scope",
      "gate": {
        "type": "build | typecheck | lint | test",
        "command": "command to run as gate check",
        "blocking": true
      },
      "tasks": [
        {
          "id": "P1.1",
          "name": "Task name — single deliverable",
          "layer": "domain | ports | primary | secondary",
          "description": "What to implement, acceptance criteria, key decisions",
          "deps": ["P0.3"],
          "files": ["path/to/file1.rs", "path/to/file2.ts"],
          "agent": "hex-coder | planner | integrator"
        }
      ]
    }
  ]
}
```

### Required Fields

| Field | Purpose |
|-------|---------|
| `id` | Unique identifier, prefixed `wp-` |
| `feature` | Display name shown in `hex plan list` |
| `adr` | ADR reference (required by ADR-050 pipeline) |
| `phases` | Array of execution phases |
| `phases[].id` | Phase identifier (P0, P1, P2...) |
| `phases[].tasks` | Array of tasks within the phase |
| `phases[].tasks[].id` | Task identifier (P1.1, P1.2...) |
| `phases[].tasks[].name` | What the task delivers |
| `phases[].tasks[].layer` | Hex layer: domain, ports, primary, secondary |

### Optional Fields

| Field | Purpose |
|-------|---------|
| `priority` | `P0-BLOCKER` blocks other work, `high`/`normal` for prioritization |
| `blocks` | Array of strings describing what this workplan blocks |
| `phases[].tier` | Dependency tier (0=domain, 1=secondary, 2=primary, 3=usecases, 4=integration) |
| `phases[].gate` | Build/test gate that must pass before next phase |
| `phases[].tasks[].deps` | Task IDs this task depends on |
| `phases[].tasks[].files` | Files this task will create or modify |
| `phases[].tasks[].agent` | Which agent role should execute this task |

## Tier System

Tiers enforce the hexagonal architecture inside-out development pattern:

| Tier | Layer | What | Depends On |
|------|-------|------|------------|
| 0 | Domain + Ports | Pure types, trait interfaces | Nothing |
| 1 | Secondary Adapters (driven) | Implementations of ports | Tier 0 |
| 2 | Primary Adapters (driving) | CLI, routes, dashboard | Tier 0 |
| 3 | Use Cases | Business logic composing ports | Tiers 0-2 |
| 4 | Composition Root + Integration | Wiring, DI, e2e tests | Everything |

### Parallelism Rules

- Tasks WITHIN a phase can run in parallel if they have no inter-task deps
- Phases are sequential (tier N must complete before tier N+1)
- Each parallel task gets its own background agent

## How Swarms Execute Workplans

### 1. Initialize Swarm
```
hex swarm init <feature-name> --topology hierarchical
```
Creates a HexFlo swarm registered in SpacetimeDB.

### 2. Create Tasks from Workplan
For each task in each phase, create a HexFlo task:
```
hex task create <swarm-id> "P1.1: Define port interfaces"
```
Tasks start as `pending`. HexFlo tracks them in SpacetimeDB.

### 3. Execute Phases In Order
```
Phase P1 (tier 0) → spawn parallel agents (one per task)
  wait for all P1 tasks to complete
  run gate check (if defined)
Phase P2 (tier 1) → spawn parallel agents
  wait for all P2 tasks to complete
  run gate check
...
```

Each agent:
- Is spawned with `HEXFLO_TASK:{task_id}` in the prompt (ADR-048)
- SubagentStart hook auto-assigns the task
- Works on the specified files
- SubagentStop hook auto-completes the task

### 4. Gate Checks
After all tasks in a phase complete, run the gate command:
```json
"gate": {
  "type": "typecheck",
  "command": "cd hex-nexus/assets && npx tsc --noEmit",
  "blocking": true
}
```
If `blocking: true` and the gate fails, the workplan halts.

### 5. Verify Completion
After all phases complete:
- All gate checks passed
- `hex plan list` shows `N/N` for the workplan
- `hex swarm list` shows `N/N done` for the swarm

## Naming Conventions

| Element | Convention | Example |
|---------|-----------|---------|
| Workplan file | `feat-<feature-name>.json` | `feat-frontend-hexagonal-architecture.json` |
| Workplan ID | `wp-<feature-name>` | `wp-frontend-hexagonal-architecture` |
| Swarm name | `adr-NNN-<feature-slug>` | `adr-056-frontend-hex` |
| Phase ID | `P{N}` | `P1`, `P2`, `P3` |
| Task ID | `P{N}.{M}` | `P1.1`, `P2.3` |
| Task title in swarm | `P{N}.{M}: {name}` | `P1.1: Create port interfaces` |

## Workplan Supersession

When a workplan's scope is fully absorbed by another:

```json
{
  "supersedes": "wp-older-plan-id",
  "supersession_reason": "Previous workplan fixed symptoms; this fixes root cause"
}
```

## Pipeline Enforcement (ADR-050)

Workplans require an ADR reference. The lifecycle pipeline is:

```
ADR → Workplan → HexFlo Swarm → Agent Work → Completion
```

Use `hex plan create <name> --adr ADR-NNN` to create a workplan.
Use `--no-adr` only for exploratory or emergency work.

## Blocker Annotation

If a workplan blocks other development, add:

```json
{
  "priority": "P0-BLOCKER",
  "blocks": [
    "All swarm-based feature development",
    "HexFlo task lifecycle"
  ]
}
```

This shows in `hex plan list` with a red priority tag and is stored in project memory so all agents are aware.

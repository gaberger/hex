---
name: workplan-format
description: Defines the hex workplan JSON format and how workplans guide HexFlo swarm execution. Always loaded when planning features or coordinating swarms.
always_load: true
---

# Workplan Format — Swarm Execution Guide

A workplan is a JSON document in `docs/workplans/` that decomposes a feature into dependency-ordered tiers of tasks. Each task maps to one hexagonal adapter boundary. HexFlo swarms execute workplans by creating tasks from the plan and assigning agents to them.

## Workplan Schema

```json
{
  "feature": "Human-readable feature name",
  "adr": "ADR-NNN",
  "created": "YYYY-MM-DD",
  "status": "planned | in_progress | complete",
  "topology": "hierarchical | mesh | adaptive",
  "budget": "~NNNNN tokens",
  "phases": 5,
  "totalSteps": 13,
  "description": "One-line summary of what this workplan delivers",

  "tiers": {
    "T0": {
      "name": "Tier name (what this tier delivers)",
      "parallel": true,
      "dependsOn": [],
      "steps": [
        {
          "id": "T0-1",
          "title": "What this step produces",
          "adapter": "layer/adapter-name",
          "files": ["path/to/file1.rs", "path/to/file2.rs"],
          "dependencies": ["crate-name"],
          "status": "todo | in_progress | done | blocked",
          "notes": "Implementation details, key decisions, gotchas"
        }
      ]
    },
    "T1": {
      "name": "Next tier",
      "parallel": false,
      "dependsOn": ["T0"],
      "steps": [...]
    }
  },

  "dependencies": {
    "cargo": [
      { "name": "crate", "version": "1.0", "purpose": "why" }
    ]
  },

  "riskRegister": [
    { "risk": "description", "impact": "high|medium|low", "mitigation": "plan" }
  ],

  "mergeOrder": [
    "T0-1 → T0-2 (domain then ports)",
    "T1-1, T1-2 parallel → T1-3 last"
  ],

  "successCriteria": [
    "Observable outcome that proves the feature works"
  ]
}
```

## Tier System

Tiers enforce the hexagonal architecture inside-out development pattern:

| Tier | Layer | What | Depends On |
|------|-------|------|------------|
| T0 | Domain + Ports | Pure types, trait interfaces | Nothing |
| T1 | Secondary Adapters (driven) | Implementations of ports | T0 |
| T2 | Primary Adapters (driving) | CLI, routes, dashboard | T0 |
| T3 | Use Cases | Business logic composing ports | T0, T1 |
| T4 | Composition Root | Wiring, DI, feature flags | T1, T2, T3 |
| T5 | Integration Tests | End-to-end verification | Everything |

### Parallelism Rules

- Steps WITHIN a tier can run in parallel if `"parallel": true`
- Steps ACROSS tiers are sequential (must wait for dependsOn tiers)
- Each parallel step gets its own git worktree (or background agent)

## How Swarms Execute Workplans

### 1. Initialize Swarm
```
hex swarm init <feature-name> --topology hierarchical
```
Creates a HexFlo swarm registered in SpacetimeDB.

### 2. Create Tasks from Workplan
For each step in the workplan, create a HexFlo task:
```
hex task create <swarm-id> "T0-1: Define domain types"
```
Tasks start as `pending`. HexFlo tracks them in SpacetimeDB.

### 3. Execute Tiers In Order
```
T0 steps → spawn parallel agents (one per step)
  wait for all T0 to complete
T1 steps → spawn parallel agents
  wait for all T1 to complete
...
```

Each agent:
- Claims its task (`swarm_task_assign`)
- Works in an isolated worktree
- Runs cargo check to verify compilation
- Marks task complete (`swarm_task_complete`) with commit hash

### 4. Merge Order
After all tasks in a tier complete, merge worktrees in the order specified by `mergeOrder`:
- Domain → ports → adapters → use cases → composition root
- Run integration tests after each merge

### 5. Verify Success Criteria
Each criterion in `successCriteria` must be verified:
- Compilation passes
- Tests pass
- hex analyze shows no new violations
- Behavioral specs match (if defined)

## Naming Conventions

| Element | Convention | Example |
|---------|-----------|---------|
| Workplan file | `feat-<feature-name>.json` | `feat-remote-agent-transport.json` |
| Swarm name | `feat-<feature-name>` | `feat-remote-agent-transport` |
| Task title | `T{tier}-{step}: {description}` | `T0-1: Define domain types` |
| Worktree branch | `feat/<feature>/<layer>` | `feat/remote-agent/domain` |

## Step Status Lifecycle

```
todo → in_progress → done
              ↓
           blocked → in_progress → done
```

A step is `blocked` when its dependsOn tier has incomplete steps.

## Creating a Workplan

When asked to plan a feature:

1. **Read the ADR** — understand the architecture decisions
2. **Decompose inside-out** — domain first, adapters second, use cases third
3. **Identify parallelism** — which steps in a tier are independent?
4. **Estimate budget** — tokens per step (typically 3K-5K per adapter)
5. **Define success criteria** — observable outcomes, not implementation details
6. **Write risk register** — what could go wrong, how to mitigate

Use `hex plan` to auto-generate a workplan from requirements, then refine manually.

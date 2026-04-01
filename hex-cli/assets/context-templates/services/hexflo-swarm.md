# HexFlo Memory — Swarm Scope

Access the swarm-scoped memory store for the current swarm. All agents in this swarm share read and write access. Use this for coordination data, phase progress, and inter-agent decisions.

## Tools

```
mcp__hex__hex_hexflo_memory_store    — write a key-value pair (scope: swarm:<id>)
mcp__hex__hex_hexflo_memory_retrieve — read by exact key
mcp__hex__hex_hexflo_memory_search   — fuzzy search within this swarm's keys
mcp__hex__hex_hexflo_task_list       — list tasks registered to this swarm
mcp__hex__hex_hexflo_swarm_status    — check swarm topology and agent states
```

## When to Use Swarm Memory

- Recording decisions made during this swarm (e.g. which approach was chosen)
- Communicating results from one phase to the next
- Tracking which tasks have been claimed or completed
- Sharing intermediate outputs (file paths, function names) between parallel agents

## Key Naming Convention

Use `<phase>/<agent-role>/<descriptor>` within the swarm namespace:

```
phase2/hex-coder/port-interface-path
phase3/hex-reviewer/violations-found
decisions/approach-chosen
blockers/unresolved
```

## Coordination Protocol

1. **Before starting a task**: read swarm memory for prior agent decisions
2. **After completing a task**: write your result + the commit hash
3. **On blocking**: write to `blockers/<your-role>` so the orchestrator can intervene
4. **On phase transition**: write a phase summary to `phase<N>/summary`

## Prior Swarm Context

{{hexflo_memory}}

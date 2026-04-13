# HexFlo Memory — Global Scope

Access the global key-value memory store shared across all agents, swarms, and projects in the hex ecosystem. Use this for decisions and context that transcend any single swarm or task.

## Tools

```
mcp__hex__hex_hexflo_memory_store    — write a key-value pair (scope: global)
mcp__hex__hex_hexflo_memory_retrieve — read by exact key
mcp__hex__hex_hexflo_memory_search   — fuzzy search across all global keys
```

## When to Use Global Memory

- Cross-project architectural decisions that all agents should know
- Shared infrastructure state (e.g. "nexus running on port 5555")
- Conventions agreed on across multiple swarms
- Resolved blockers that future swarms should not re-investigate

## Key Naming Convention

Use `<project>/<category>/<descriptor>` for discoverability:

```
<<<<<<< HEAD
{{project_name}}/arch/core-rules-summary
{{project_name}}/infra/service-port
{{project_name}}/decision/database-name
=======
<project>/arch/hexagonal-rules-summary
<project>/infra/service-port
<project>/decision/database-name
>>>>>>> worktree-agent-aacb2365
```

## What NOT to Store Here

- Per-swarm coordination data → use swarm-scoped memory
- Per-agent working state → use agent-scoped memory
- Secrets or credentials → use `mcp__hex__hex_secrets_vault_set`
- Large blobs (AST dumps, full file contents) — store a pointer, not the content

## Prior Global Context

{{hexflo_memory}}

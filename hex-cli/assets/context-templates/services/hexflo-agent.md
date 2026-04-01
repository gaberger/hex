# HexFlo Memory — Agent Scope

Access your personal key-value memory store. This memory is private to your agent instance and persists across tool calls within this session. Use it to track your own progress and intermediate state.

## Tools

```
mcp__hex__hex_hexflo_memory_store    — write a key-value pair (scope: agent:<id>)
mcp__hex__hex_hexflo_memory_retrieve — read by exact key
mcp__hex__hex_hexflo_memory_search   — fuzzy search your personal keys
```

## When to Use Agent Memory

- Checkpointing progress within a multi-step task (so you can resume after an error)
- Storing intermediate results you'll need later in the same task
- Tracking which files you've read, which tests you've run
- Recording what you tried that didn't work (to avoid retrying)

## Key Naming Convention

Use short, flat keys — this store is private, no namespacing needed:

```
current-file
last-test-result
files-modified
approach-tried
task-step
```

## Checkpoint Pattern

Write a checkpoint before any operation that could fail or take a long time:

```
checkpoint/before-refactor  →  "ports/prompt.rs at line 87, approach: extract trait"
checkpoint/after-compile    →  "PASS — cargo check clean"
```

This lets you recover without re-reading files you've already processed.

## What NOT to Store Here

- Results that other agents need → use swarm-scoped memory
- Decisions that future sessions should know → use memory-extraction service
- Large file contents → store only the path and the relevant line range

# Session Memory

Maintain context across this session using structured working notes. Update these notes as your understanding evolves — they are your primary tool for staying coherent across long tasks.

## Working Note Format

Keep a single working note updated throughout the session. Structure it as:

```
Title: <5-10 word description of the current task>
Project: {{project_name}} | Phase: {{current_phase}}

Current State:
  What is actively in progress right now.

Task Specification:
  What was asked. Do not paraphrase — capture the intent verbatim.

Files & Functions:
  - path/to/file.rs — why it matters (e.g. "owns the port trait for X")
  - Only include files you have actually read or modified.

Workflow:
  Ordered commands to reproduce the current state:
  1. cargo build -p hex-cli
  2. hex analyze .

Errors & Corrections:
  - What failed → how it was fixed. Include error messages verbatim.

Learnings:
  - What worked well.
  - What to avoid repeating.
```

## When to Update

- After reading a new file that changes your understanding
- After each tool call that produces a non-obvious result
- Immediately after fixing a bug or resolving a compile error
- Before spawning a background agent (so it can inherit context)

## What NOT to Store Here

- Code snippets (read the file directly when needed)
- Git history (use `git log`)
- Information that belongs in persistent memory — use the memory-extraction service for that

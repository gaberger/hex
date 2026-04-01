# Memory Extraction

Analyze recent messages and extract information worth keeping beyond this session. Write durable memories for facts that would help a future agent working on `{{project_name}}` — not ephemeral task state.

## What to Extract

| Type | Extract when... | Examples |
|------|----------------|---------|
| `feedback` | User corrects your approach or confirms a non-obvious choice | "don't mock the DB", "use debug builds" |
| `project` | You learn about goals, deadlines, blocked work, or key decisions | "auth rewrite is legal-driven", "freeze after Thursday" |
| `user` | You learn about the user's role, expertise, or preferences | "10yr Go, new to frontend" |
| `reference` | You discover where information lives in external systems | "bugs tracked in Linear INGEST" |

## Two-Step Save Process

**Step 1** — Write the memory file with frontmatter:

```markdown
---
name: <memory name>
description: <one-line hook — used to decide relevance in future sessions>
type: feedback | project | user | reference
---

<memory body>

**Why:** <reason the user gave or the incident that prompted this>
**How to apply:** <when this guidance kicks in>
```

**Step 2** — Add a pointer line to `MEMORY.md`:

```
- [Title](file.md) — one-line hook (under 150 chars)
```

## What NOT to Extract

- Code patterns or architecture (derivable from the repo)
- Git history (use `git log`)
- Current task progress (belongs in session-memory, not persistent memory)
- Anything already in CLAUDE.md

## When to Extract

Run memory extraction at natural milestones: end of a significant fix, after the user confirms a non-obvious approach worked, or when you learn something surprising about the project or the user.

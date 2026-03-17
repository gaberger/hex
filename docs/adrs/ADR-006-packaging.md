# ADR-006: Skills, Agent Definitions, and npm Packaging

**Status:** Accepted
**Date:** 2026-03-15

## Context

hex produces reusable artifacts: project scaffolds, code generation workflows, and agent configurations. These need to be distributable in formats that match how consumers adopt them — from casual one-off use (skill trigger) to full integration (npm package).

## Decision

We support **three packaging formats**, each targeting a different adoption level.

### 1. Claude Skill Markdown

For end users who invoke capabilities via natural language triggers. Skills are Markdown files (`.md`) with YAML frontmatter for metadata.

```markdown
---
name: hex-scaffold
description: Scaffold a new hexagonal architecture project
triggers: ["create hex project", "scaffold hexagonal", "new ports and adapters"]
---

# hex-scaffold
Prompt content loaded when triggered...
```

**Skill triggers** use progressive disclosure: the trigger phrase loads a minimal prompt, which then loads additional context only if the user's request requires it. This keeps the initial token cost near zero.

### 2. Agent Definition YAML

For Ruflo swarm orchestration. Defines agent roles with context loading strategies.

```yaml
name: hex-coder
type: coder
context:
  L1: all port summaries
  L2: target adapter signatures
  L3: file being edited
constraints:
  - implement port interface fully
  - include unit tests
```

**Context loading strategy** (L1 to L2 to L3):

| Level | When Loaded | Token Cost | Contains |
|-------|------------|-----------|----------|
| **L1** | Agent start | ~50/file | Exports, imports, deps |
| **L2** | Task assignment | ~200/file | Full type signatures |
| **L3** | Active editing | ~2000/file | Complete source |

Agents start with L1 summaries of the full project, drill to L2 for their assigned adapter's neighbors, and load L3 only for the file they are editing. This caps context at ~5000 tokens for a typical adapter task.

### 3. npm Package

For programmatic integration into CI/CD pipelines and developer toolchains.

```bash
npx hex init --lang typescript --name my-project
npx hex generate --adapter secondary/database --from spec.md
npx hex summarize --level L2 --output context.txt
```

The npm package exposes the full port interface programmatically, enabling custom adapters and build integrations.

### Versioning and Distribution

- **Semver** for all three formats, tracked in a single `version` field in `package.json`
- Skills and agent definitions are embedded in the npm package under `skills/` and `agents/`
- npm is the sole distribution channel; skills (Markdown) and agent definitions (YAML) are extracted at install time
- Breaking port interface changes require a major version bump
- Agent definitions pin a minimum hex version to ensure port compatibility

## Consequences

### Positive

- Three adoption tiers lower the barrier to entry (trigger phrase vs full npm install)
- Single source of truth for versioning avoids format drift
- L1-L2-L3 context loading keeps agent token costs predictable and low
- npm as sole channel simplifies CI/CD and dependency management

### Negative

- Markdown skill format is Claude-specific; other LLM platforms need their own format
- Embedding skills inside npm means non-Node consumers must extract manually
- Agent definitions couple to Ruflo's task model; alternative orchestrators need adapters

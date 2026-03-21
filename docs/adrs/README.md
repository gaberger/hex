# Architecture Decision Records (ADRs)

## What are ADRs?

Architecture Decision Records capture important architectural decisions along with their context and consequences. They are immutable records of WHY decisions were made, not just WHAT was decided.

## Directory Structure

Every hex project stores ADRs in `docs/adrs/`:

```
project-root/
└── docs/
    └── adrs/
        ├── README.md          ← This file
        ├── TEMPLATE.md        ← Template for new ADRs
        ├── ADR-001-*.md       ← First decision
        ├── ADR-002-*.md       ← Second decision
        └── ...
```

## File Naming Convention

```
ADR-{NNN}-{kebab-case-slug}.md
```

- `NNN`: Three-digit zero-padded number (001, 002, ..., 043)
- `slug`: Lowercase kebab-case summary of the decision
- Extension: Always `.md`

Examples:
- `ADR-001-hexagonal-architecture.md`
- `ADR-025-spacetimedb-state-backend.md`
- `ADR-043-aiide-hex-nexus.md`

## Status Lifecycle

```
Proposed → Accepted → [Deprecated | Superseded]
```

| Status | Meaning |
|--------|---------|
| **Proposed** | Under discussion, not yet adopted |
| **Accepted** | Active decision, currently in effect |
| **Deprecated** | No longer relevant (technology changed, feature removed) |
| **Superseded** | Replaced by a newer ADR (link to successor) |

ADRs are **never deleted**. If a decision changes, write a new ADR that supersedes the old one.

## Required Frontmatter

Every ADR must begin with:

```markdown
# ADR-{NNN}: {Title}

**Status:** {Proposed | Accepted | Deprecated | Superseded}
**Date:** {YYYY-MM-DD}
**Drivers:** {What triggered this decision}
```

Optional frontmatter:
```markdown
**Supersedes:** ADR-{NNN}
**Superseded by:** ADR-{NNN}
```

## Required Sections

1. **Context** — The problem, forces, constraints, alternatives considered
2. **Decision** — The chosen approach (imperative language)
3. **Consequences** — Positive, negative, and mitigations

## Optional Sections

- **Implementation** — Phased rollout plan with status tracking
- **References** — Links to related ADRs, issues, documents

## Creating a New ADR

### Via CLI
```bash
hex adr create "Title of Decision"
```

### Via Dashboard
Navigate to `Configuration > ADRs > + New ADR` in Hex Nexus.

### Manually
1. Copy `TEMPLATE.md`
2. Rename to `ADR-{next-number}-{slug}.md`
3. Fill in all required sections
4. Set status to `Proposed`
5. Commit to the repository

## For AI Inference Engines

When an AI agent needs to make an architectural decision:

1. **Check existing ADRs** — Search for related decisions first: `hex adr search <keyword>`
2. **Follow the template** — Use `TEMPLATE.md` exactly. Do not invent custom formats.
3. **Reference predecessors** — If this decision relates to or supersedes an existing ADR, link it.
4. **Be specific** — Include concrete file paths, module names, and API contracts in the Decision section.
5. **Track implementation** — Use the Implementation table to show progress.

### Parsing ADRs Programmatically

ADR metadata is extracted from the markdown content:
- **Title**: First line starting with `# ` (strip `ADR-NNN: ` prefix)
- **Status**: Line containing `**Status:**` (extract value after colon)
- **Date**: Line containing `**Date:**` (extract value after colon)
- **Drivers**: Line containing `**Drivers:**` (extract value after colon)

### Status Badge Colors (Hex Nexus Dashboard)
- Proposed: yellow (`#eab308`)
- Accepted: green (`#4ade80`)
- Deprecated: gray (`#6b7280`)
- Superseded: red (`#f87149`)

## API Access

When hex-nexus is running:
- `GET /api/adrs` — List all ADRs with parsed metadata
- `GET /api/adrs/{NNN}` — Get a single ADR's full markdown content

## Hex Architecture Integration

ADRs are enforced by the hex architecture analysis:
- `hex analyze .` checks for boundary violations against ADR rules
- The Health ring in the dashboard reflects ADR compliance
- Agents reference ADRs when making code changes

## Example: Creating an ADR for a New Feature

```markdown
# ADR-044: Feature X Implementation Strategy

**Status:** Proposed
**Date:** 2026-03-22
**Drivers:** User request for feature X, performance constraints from ADR-028

## Context

We need to implement feature X. The current architecture (ADR-001) uses
hexagonal patterns. ADR-025 requires all state to flow through SpacetimeDB.

## Decision

Implement feature X as a secondary adapter behind the `IFeatureXPort`
interface. Use the inference-gateway (ADR-035) for LLM calls.

## Consequences

**Positive:**
- Clean separation via ports
- Testable with dependency injection (ADR-014)

**Negative:**
- Additional adapter complexity

**Mitigations:**
- Use hex-coder agent with TDD to ensure coverage
```

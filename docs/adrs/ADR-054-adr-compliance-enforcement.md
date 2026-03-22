# ADR-054: ADR Compliance Enforcement — Preventing Architectural Drift

**Status:** Proposed
## Date: 2026-03-21

## Context

hex has 44+ Architecture Decision Records covering everything from hexagonal boundaries (ADR-001) to SpacetimeDB as the state backend (ADR-039). However, no automated mechanism enforces these decisions. Architectural drift happens silently:

- ADR-039 says nexus is stateless compute, but `state.projects` is an in-memory HashMap
- ADR-001 says adapters must not import other adapters, but `hex analyze` only checks import paths
- New code gets written against the *codebase* not the *ADRs*, so decisions are forgotten

This is especially dangerous in AI-driven development where agents generate code at high velocity without reading ADR context.

## Decision

### Layer 1: ADR Compliance Rules in `hex analyze`

Add a new analysis pass that checks code against accepted ADRs. Each rule maps to a specific ADR and produces violations with the same format as boundary violations:

```
WARN [ADR-039] routes/git.rs:30 — reads from state.projects HashMap (should use SpacetimeDB project table)
WARN [ADR-001] adapters/primary/cli.rs:15 — imports from adapters/secondary (cross-adapter coupling)
```

**Initial rule set:**

| Rule ID | ADR | What It Checks |
|---------|-----|----------------|
| `adr-001-no-cross-adapter` | ADR-001 | Adapters must not import other adapters |
| `adr-001-domain-purity` | ADR-001 | Domain layer has zero external imports |
| `adr-014-no-mock-module` | ADR-014 | Tests must not use `mock.module()` |
| `adr-039-no-rest-state` | ADR-039 | REST handlers must not be source of truth for state |
| `adr-039-spacetimedb-first` | ADR-039 | If a SpacetimeDB table exists for data X, code must read from SpacetimeDB, not REST HashMap |

### Layer 2: ADR Frontmatter Enforcement Tags

ADRs gain an optional `enforced_by` frontmatter field that names the analysis rule:

```markdown
# ADR-039: Nexus Agent Control Plane
**Status:** Accepted
## Enforced-By: adr-039-no-rest-state, adr-039-spacetimedb-first
```

When `hex adr list` runs, it shows which ADRs have enforcement and which are "honor system only."

### Layer 3: Pre-Commit Hook

`hex analyze --adr-compliance` runs as a pre-commit hook. Violations produce warnings (not errors, to avoid blocking agents). A `--strict` flag promotes warnings to errors for CI.

### Layer 4: Agent Context Injection

When spawning agents for code generation, the agent's prompt includes relevant ADR summaries extracted from `hex adr search <topic>`. This prevents drift at generation time, not just detection time.

## Consequences

### Positive
- Architectural decisions are enforced, not just documented
- New contributors (human and AI) get immediate feedback on violations
- ADR compliance is visible in the health score

### Negative
- Rules need maintenance as ADRs evolve
- False positives may slow agents initially
- Requires ADR authors to write enforcement rules (extra effort)

## Implementation Notes

### Nexus-Specific Rules (ADR-039)

For the current git integration, the rule `adr-039-spacetimedb-first` would flag:
- `state.projects.read().await` in any route handler as a WARN
- The recommended fix: query SpacetimeDB `project` table via `state_port`, or accept path from frontend

The exception list includes:
- `state.commands` / `state.results` — ephemeral command dispatch, not persistent state
- `state.ws_tx` — WebSocket broadcast channel, not state
- `state.hexflo` — HexFlo is the coordination layer, wraps SpacetimeDB

## State Persistence (SpacetimeDB)

ADR compliance results are **shared state** — remote agents working on the same project need to see violations. Per ADR-039, this belongs in SpacetimeDB, not in REST memory.

### Current Implementation (HexFlo Memory)

Results are stored in HexFlo's key-value memory store (backed by SpacetimeDB `hexflo_memory` table):

```
Key:   adr-compliance:{project_id}
Scope: compliance
Value: JSON { violationCount, errorCount, warningCount, violations[], checkedAt }
```

Remote agents can read compliance state via:
- `hex memory get adr-compliance:{project_id}` (CLI)
- `mcp__hex__hex_hexflo_memory_retrieve` (MCP tool)
- SpacetimeDB subscription on `hexflo_memory` table (dashboard)

### Future: Dedicated SpacetimeDB Table

When the SpacetimeDB module is updated, migrate to a proper table:

```rust
#[spacetimedb(table)]
pub struct AdrViolation {
    #[primarykey]
    pub id: String,             // UUID
    pub project_id: String,     // FK to project table
    pub adr: String,            // "ADR-039"
    pub rule_id: String,        // "adr-039-no-rest-state"
    pub file: String,
    pub line: u32,
    pub message: String,
    pub severity: String,       // "error" | "warning" | "info"
    pub checked_at: String,     // ISO 8601
    pub resolved_at: String,    // Empty until fixed
}
```

This enables:
- Real-time subscription: dashboard updates instantly when violations change
- Historical tracking: when was a violation introduced/resolved?
- Cross-agent coordination: agent A sees violations found by agent B
- Compliance trends: violation count over time per ADR

### Rule File Location

Rules live in the **project**, not the framework:

```
{project_root}/.hex/adr-rules.toml
```

hex ships the compliance **engine** (pattern matcher + file scanner).
The project ships the **rules** (which patterns violate which ADRs).

## References

- ADR-001: Hexagonal Architecture
- ADR-014: No mock.module()
- ADR-039: Nexus Agent Control Plane
- ADR-041: ADR Review Agent
- ADR-044: Git Integration

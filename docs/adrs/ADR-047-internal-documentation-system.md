# ADR-047: Internal Documentation System

## Status

Proposed

## Context

hex has grown to 5 deployment units, 18 SpacetimeDB WASM modules, 7 Rust crates, a TypeScript library with 31 port interfaces, 37 ADRs, 14 agent definitions, and 6 skills. Documentation is currently scattered across:

- **README.md** — public-facing overview (1000+ lines, mixes marketing with technical)
- **CLAUDE.md** — model-facing instructions (originally Claude-specific, now generalized)
- **ADRs** — architectural decisions (37 files, no cross-referencing standard)
- **Inline code comments** — inconsistent across Rust, TypeScript, and WASM modules
- **Skill files** — embedded documentation for slash commands
- **Agent YAML** — role descriptions in agent definitions

### Problems

1. **No canonical glossary** — "hex-nexus" has been called "hex-hub", "daemon", "orchestration nexus", and "filesystem bridge" in different files. "hex-agent" has been confused with the hexagonal "adapter" concept, and old names like "hex-hub" persist across docs.

2. **No component documentation** — Each of the 5 deployment units (SpacetimeDB, hex-nexus, hex-agent, hex-dashboard, hex clients) lacks a self-contained document explaining its purpose, API surface, configuration, and relationship to other components.

3. **WASM module contracts are undocumented** — 18 SpacetimeDB modules define tables and reducers that are the backbone of the system, but their contracts (what reducers expect, what subscriptions return) exist only in source code.

4. **Multiple consumers, one format** — Developers, AI agents, and MCP tools all need documentation but in different shapes:
   - Developers need architectural guides with rationale
   - AI agents need token-efficient summaries with typed contracts
   - MCP tools need structured metadata (JSON schemas)

5. **No documentation linting** — Stale references (e.g., to "ruflo", "hex-hub", "hex-agent") persist because nothing checks doc accuracy against code.

6. **ADR cross-referencing is manual** — ADRs reference each other by number but there's no dependency graph. When an ADR is superseded, dependent ADRs may not be updated.

## Decision

Introduce a structured internal documentation system with three tiers, a canonical glossary, and automated freshness checking.

### Tier 1: Canonical Reference (Single Source of Truth)

Create `docs/reference/` as the authoritative documentation root:

```
docs/reference/
├── glossary.md              # Canonical terminology (MUST be used everywhere)
├── system-architecture.md   # 5 deployment units, their relationships, data flow
├── components/
│   ├── spacetimedb.md       # SpacetimeDB role, modules, connection patterns
│   ├── hex-nexus.md         # Filesystem bridge: REST API, config sync, dashboard serving
│   ├── hex-agent.md       # Enforcement runtime: skills, hooks, dispatchers
│   ├── hex-dashboard.md     # Control plane: views, WebSocket subscriptions, commands
│   └── hex-clients.md       # CLI, web, desktop, chat — connection and capabilities
├── modules/
│   ├── hexflo-coordination.md  # Tables, reducers, subscription queries
│   ├── agent-registry.md       # Agent lifecycle contract
│   ├── inference-gateway.md    # Request routing contract
│   └── ...                     # One file per WASM module
├── ports/
│   ├── state-port.md           # IStatePort — dual backend abstraction
│   ├── coordination-port.md    # ICoordinationPort — multi-instance locking
│   └── ...                     # Key port interfaces with usage examples
└── deployment/
    ├── local-dev.md            # Setting up a local dev environment
    ├── production.md           # Production deployment topology
    └── troubleshooting.md      # Common issues and diagnostics
```

### Tier 2: Decision Records (ADRs)

ADRs continue in `docs/adrs/` but with improved structure:

1. **Required frontmatter** — Every ADR must include:
   ```yaml
   ---
   id: ADR-047
   status: proposed | accepted | deprecated | superseded | rejected
   date: 2026-03-21
   supersedes: []          # ADR IDs this replaces
   superseded_by: null     # ADR ID that replaces this
   depends_on: []          # ADR IDs this builds upon
   components: []          # Which deployment units this affects
   modules: []             # Which WASM modules this affects
   ---
   ```

2. **Component tagging** — Every ADR must list which components it affects (spacetimedb, hex-nexus, hex-agent, hex-dashboard, hex-clients). This enables filtering: "show me all ADRs affecting hex-nexus."

3. **ADR dependency graph** — `hex adr graph` command generates a visual dependency graph from frontmatter.

### Tier 3: Inline & Generated Documentation

1. **Rust doc comments** — All public APIs in Rust crates must have `///` doc comments. `cargo doc` generates browsable HTML.

2. **WASM module READMEs** — Each `spacetime-modules/<module>/` gets a `README.md` documenting:
   - Tables (schema, indexes)
   - Reducers (parameters, side effects, error cases)
   - Subscription queries (what clients should subscribe to)
   - Example usage

3. **TypeScript port JSDoc** — All port interfaces in `src/core/ports/` must have JSDoc with `@example` blocks.

### Glossary Enforcement

The glossary (`docs/reference/glossary.md`) is the canonical terminology source. Key entries:

| Term | Definition | NOT |
|------|-----------|-----|
| **hex** | AI-Assisted Integrated Development Environment (AAIDE) | "harness", "framework" alone |
| **hex-nexus** | Filesystem bridge daemon — bridges SpacetimeDB WASM sandbox ↔ local OS | "hub", "orchestration nexus", "daemon" alone |
| **hex-agent** | Architecture enforcement runtime — agent runtime for AI dev agents (local/remote) | Do not confuse with hexagonal "adapter" concept |
| **hex-dashboard** | Developer control plane for multi-project management | "dashboard" alone (ambiguous) |
| **SpacetimeDB** | Coordination & state core — required backbone service | "database" alone (undersells its role) |
| **WASM module** | SpacetimeDB server-side logic (tables + reducers) | "plugin", "extension" |
| **reducer** | Transactional stored procedure in a WASM module | "endpoint", "handler" |
| **HexFlo** | Native Rust swarm coordination layer in hex-nexus | "ruflo" (predecessor, deprecated) |
| **port** | Typed interface contract between architecture layers | "API", "service" |
| **adapter** | Implementation of a port for a specific technology | "plugin", "driver" |
| **composition root** | Single file that wires adapters to ports (DI point) | "config", "bootstrap" |

### Documentation Freshness

1. **`hex docs check`** — New CLI command that:
   - Scans `docs/reference/` for references to code symbols (functions, types, modules)
   - Verifies those symbols still exist in the codebase
   - Flags stale terminology (checks against glossary)
   - Reports docs with no git activity in 90+ days

2. **Pre-commit hook** — When files in `spacetime-modules/` change, warn if the corresponding `docs/reference/modules/` doc wasn't updated.

3. **ADR staleness** — Existing `hex adr abandoned` command extended to check `depends_on` chains for cascading staleness.

### AI Agent Documentation Format

For AI agent consumption, `docs/reference/` files follow a structure optimized for token efficiency:

```markdown
# Component: hex-nexus

## One-Line Summary
Filesystem bridge daemon — bridges SpacetimeDB WASM sandbox ↔ local OS.

## Key Facts
- REST API at port 5555
- Serves dashboard frontend (rust-embed)
- Syncs config files → SpacetimeDB on startup (ADR-044)
- Requires SpacetimeDB to be running

## API Surface
[Concise table of endpoints]

## Configuration
[Environment variables and config files]

## Depends On
- SpacetimeDB (coordination & state)
- hex-core (shared types)

## Depended On By
- hex-cli (delegates commands)
- hex-dashboard (served by this binary)
- hex-agent (filesystem operations)
```

This format gives AI agents the key facts in ~100 tokens, with structured sections for deeper exploration.

## Consequences

### Positive

- **Single source of truth** — Glossary prevents terminology drift across 5 deployment units
- **WASM module contracts documented** — Reducer signatures and table schemas are discoverable without reading Rust source
- **Multi-consumer** — Same docs serve developers, AI agents, and tooling
- **Automated freshness** — `hex docs check` catches stale references before they confuse agents
- **ADR dependency tracking** — Know which decisions cascade when one is superseded
- **Onboarding** — New contributors (human or AI) can understand the system from `docs/reference/system-architecture.md`

### Negative

- **Maintenance overhead** — More docs to keep updated (mitigated by freshness checking)
- **Migration effort** — Existing scattered documentation must be consolidated
- **Glossary discipline** — Team must commit to using canonical terms (enforcement via linting helps)

### Implementation Order

1. **Phase 1**: Create glossary + system-architecture doc (immediate — establishes terminology)
2. **Phase 2**: Component docs for all 5 deployment units (builds on README/CLAUDE.md work already done)
3. **Phase 3**: WASM module READMEs (one per module, reducer contracts)
4. **Phase 4**: `hex docs check` CLI command (automated freshness)
5. **Phase 5**: ADR frontmatter migration + dependency graph

## Dependencies

- ADR-045 (ADR Compliance Enforcement) — extends staleness detection
- ADR-044 (Config Sync) — documents sync behavior
- ADR-025 (SpacetimeDB State Backend) — foundational component documentation
- ADR-027 (HexFlo Swarm Coordination) — coordination layer documentation

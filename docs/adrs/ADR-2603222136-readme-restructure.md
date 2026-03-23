# ADR-2603222136: README Restructure — Accurate, Modular Documentation

**Status:** Proposed
**Date:** 2026-03-22
**Drivers:** README.md contains outdated information (npm installation that doesn't exist yet, npx commands that don't work, comparison tables with products that may not exist). The file is 850+ lines and mixes architecture documentation with user guides. Multiple sections reference the TypeScript CLI which has been replaced by Rust.

## Context

The current README has several problems:

1. **Inaccurate installation section** — references `npm install -g @anthropic-hex/hex` and `npx` commands that don't exist. hex is currently installed via Rust binary compilation.
2. **Outdated CLI reference** — lists commands like `hex build`, `hex orchestrate`, `hex setup`, `hex dashboard` that don't exist in the Rust CLI. Missing new commands (`hex enforce`, `hex agent audit`, `hex assets`, `hex test history`).
3. **Stale comparison table** — compares hex to SPECKit and BMAD which may confuse users.
4. **Monolithic file** — 850+ lines mixing architecture docs, user guides, API reference, and marketing content. Hard to maintain.
5. **References TypeScript CLI** — smoke test examples, Quick Start section all reference the deprecated TS CLI.
6. **Missing enforcement architecture** — the 3-layer enforcement (hooks + MCP + nexus) is only briefly mentioned.

## Decision

### Split README into focused documents

```
README.md                          # Project overview + Quick Start (< 200 lines)
docs/
  ARCHITECTURE.md                  # System components, hexagonal layers, SpacetimeDB
  CLI-REFERENCE.md                 # Complete CLI command reference (auto-generated from hex --help)
  ENFORCEMENT.md                   # 3-layer enforcement architecture (hooks + MCP + nexus)
  DEVELOPMENT.md                   # Build, test, contribute guide
  DASHBOARD.md                     # Control plane features and screenshots
  SWARM-COORDINATION.md            # HexFlo, multi-agent, worktree isolation
```

### README.md content (kept short)

1. Banner + badges
2. One-paragraph description
3. Quick Start (actual working commands with `hex` binary)
4. System architecture diagram (simplified)
5. Links to detailed docs
6. License

### Remove or fix

- Remove: npm/npx installation (not published yet)
- Remove: SPECKit/BMAD comparison table
- Remove: Inline code examples longer than 10 lines (move to docs/)
- Fix: CLI reference to match `hex --help` output
- Fix: Quick Start to use Rust binary commands
- Add: Prerequisites (Rust toolchain, SpacetimeDB)

## Implementation

| Phase | Description | Status |
|-------|------------|--------|
| P1 | Extract ARCHITECTURE.md from README system components section | Pending |
| P2 | Extract CLI-REFERENCE.md — auto-generate from hex --help | Pending |
| P3 | Extract ENFORCEMENT.md from ADR-2603221939 + ADR-2603221959 content | Pending |
| P4 | Extract DEVELOPMENT.md (build, test, contribute) | Pending |
| P5 | Rewrite README.md — short overview + links to docs/ | Pending |
| P6 | Remove stale content (npm install, comparison table, TS CLI refs) | Pending |

## Consequences

**Positive:**
- README accurately reflects what hex is and how to use it today
- Modular docs are easier to maintain as the project evolves
- New contributors aren't confused by outdated instructions
- Each doc file has a single responsibility

**Negative:**
- More files to maintain
- Links between docs need upkeep

## References

- Current README.md (850+ lines, partially inaccurate)
- ADR-055: README-Driven Project Specification
- ADR-2603221939: Mandatory Swarm Tracking (enforcement docs)
- ADR-2603221959: Provider-Agnostic Enforcement (enforcement docs)

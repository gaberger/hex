# ADR-001: Hexagonal Architecture as Foundational Pattern

**Status:** Accepted
**Verified:** 2026-05-12 via `hex analyze .` — 634 files scanned, 0 boundary violations, architecture grade A+ (100/100). The ADR's claim that domain→ports→adapters→composition-root layering is enforced is empirically true across the workspace.
## Date: 2026-03-15

## Context

AI coding agents need bounded contexts to work effectively. Without clear boundaries, agents either need too much context (token-expensive) or make cross-cutting changes that conflict with parallel agents.

## Decision

Adopt Hexagonal Architecture (Ports & Adapters) as the foundational pattern for all generated projects:

- **Domain Core** contains pure business logic with zero dependencies
- **Ports** are typed interfaces defining contracts at boundaries
- **Adapters** implement ports and are the only layer with external dependencies
- Each adapter is independently testable and assignable to a single agent

## Consequences

- **Positive**: Agents work on one adapter at a time with full context in ~200 tokens
- **Positive**: Port interfaces serve as compile-time contracts between agents' work
- **Positive**: Adapters can be swapped without touching core logic
- **Negative**: More files and interfaces than flat architecture
- **Negative**: Requires discipline to keep domain pure (enforced by lint rules)

## Enforcement

- Lint rule: domain/ cannot import from adapters/
- Lint rule: adapters/ can only import from ports/
- CI gate: dependency direction check on every PR

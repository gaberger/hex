# ADR-001: Hexagonal Architecture as Foundational Pattern

**Status:** Accepted
**Verified:** 2026-05-12 via `hex analyze .` — 634 files scanned, 0 boundary violations, architecture grade A+ (100/100). The ADR's claim that domain→ports→adapters→composition-root layering is enforced is empirically true across the workspace.
## Date: 2026-03-15 (rationale expanded 2026-05-17 under wp-hexagonal-architecture-foundation)

## Context

hex is an AIOS where the **users are AI agents** and the **sysadmins are humans**. This inverts the conventional architecture decision-axis. A pattern is justified not by its ergonomics for human engineers, but by whether it lets a fleet of autonomous agents work concurrently without stepping on each other, with bounded context windows, under deterministic verification gates.

Three forces pushed us to Hexagonal Architecture (Ports & Adapters):

1. **Context-window economics.** Frontier LLMs charge per token. An agent that needs to load 50 files to understand one change is 10× more expensive than one that loads 5. Bounded contexts collapse the load.
2. **Parallel-agent merge safety.** When N agents edit overlapping files, merges conflict and work is lost. We need a boundary-shape that *physically prevents* two agents from needing the same file for unrelated work.
3. **Deterministic verification.** Validation (compile, lint, test, property, behavioral spec) must run per-task without spinning up the entire system. Adapter-scoped tests with port mocks are O(1); end-to-end tests are O(N).

Cross-cutting changes — the kind that touch domain + several adapters + composition root in one PR — fight all three forces. So the architecture must make cross-cutting changes feel *expensive enough* that agents naturally split them along the boundaries before starting.

## Decision

Adopt Hexagonal Architecture (Ports & Adapters) as the **foundational pattern for hex itself and for every project hex scaffolds**. Concretely:

- **Domain Core** — pure business logic, zero external dependencies (no I/O, no clock, no network, no DB driver, no UI framework). Domain code is the only layer that survives a tech-stack rewrite untouched.
- **Ports** — typed interfaces (Rust traits, TypeScript interfaces) defining contracts at every boundary where the domain meets the outside world. Ports import only from `domain/` and only for value types.
- **Adapters** — implementations of ports. The **only** layer permitted to depend on external crates/libraries (axum, sqlx, fs, http, tree-sitter, …). Split into:
  - `adapters/primary/` — *driving* adapters (CLI, HTTP, browser input, MCP).
  - `adapters/secondary/` — *driven* adapters (SpacetimeDB, filesystem, git, inference providers).
- **Use cases** — application logic that *composes* ports. May import `domain/` + `ports/`, never adapters.
- **Composition root** — the **single** file that imports adapters and wires them into ports. Every project has exactly one.

Each adapter is independently testable, independently buildable, and **assignable to a single agent on a single git worktree**. This is what makes the 8-parallel-worktree feature-dev workflow ([ADR-004](ADR-004-swarm-worktrees.md)) sound.

## Rationale (why ports & adapters, not the alternatives)

| Alternative considered | Why rejected |
|---|---|
| **Layered / N-tier** | Allows downward dependencies but not the inversion. Adapter swaps still ripple through use-cases. |
| **Clean architecture (Uncle Bob)** | Strictly a superset of hex; we chose hex because the port/adapter naming maps 1:1 to "thing an agent owns end-to-end". |
| **Onion architecture** | Same dependency rule as hex; loses the symmetric primary/secondary distinction that makes CLI vs DB obvious. |
| **Flat modules + DI container** | No structural enforcement — relies on review discipline. Catastrophic for agent-authored code. |
| **Vertical slice (feature folders)** | Optimizes for human navigation of features, not for boundary-scoped parallelism. Two agents editing the same slice still collide. |

The deciding factor: **hexagonal architecture is the only pattern where "one agent = one adapter = one worktree = one merge unit" is a structurally true statement rather than a convention**.

## Consequences

### Positive
- **Bounded context per agent**: an `hex-coder` working on `adapters/secondary/spacetime/` loads the SpacetimeDB adapter + the port it implements (~200 tokens) and nothing else.
- **Compile-time contracts between agents**: port interfaces are the integration spec. If agent A's adapter satisfies the trait and agent B's use-case calls the trait, integration works without coordination.
- **Swappable adapters**: SQLite → SpacetimeDB ([ADR-025](ADR-025-spacetimedb-coordination.md) era) was a single-adapter swap; domain and use-cases were untouched.
- **Deterministic test pyramid**: domain tests are pure, port tests use trait mocks via the Deps pattern ([ADR-014](ADR-014-no-mock-module-di-deps.md)), adapter tests can hit real services. No `mock.module()` hacks, no global monkey-patching.
- **Standalone mode is trivial** (ADR-2026-04-11-2000): swap the inference adapter from Anthropic to Ollama; nothing else moves.
- **Architecture is observable**: `hex analyze .` walks the AST and grades dependency direction. The grade is real signal because the rules are real constraints.

### Negative (and how we mitigate)
- **More files than a flat layout** → mitigated by `hex scaffold` generating the boilerplate so humans rarely type it.
- **Domain purity requires discipline** → enforced mechanically: the dead-code-analyzer flags any `use` statement in `domain/` that resolves outside `domain/`.
- **New contributors face a learning curve** → CLAUDE.md states the 7 rules; `hex-feature-dev` walks through the phases with the boundary in mind.
- **Cross-adapter features feel slow** → that's the point. The friction is a feature: it forces the work to be split along the port boundary before the first line is written.

## Enforcement

The architecture is enforced at four levels, in increasing strictness:

1. **Static lint** (`hex analyze .`) — tree-sitter-based dependency graph; reports boundary violations with file:line ([ADR-2026-04-06-1000](ADR-2026-04-06-1000-treesitter-architecture-enforcement.md)).
2. **Agent runtime** (`hex-agent`) — refuses to apply diffs that introduce illegal imports.
3. **CI gate** (`hex ci`) — same checks run on every PR; blocks merge on violations.
4. **ADR compliance** ([ADR-054](ADR-054-adr-compliance-enforcement.md)) — diffs that contradict an Accepted ADR (this one, primarily) are rejected at the SOP-executor layer before reaching disk.

The 7 enforced rules (from CLAUDE.md, mechanically checked):

1. `domain/` imports only `domain/`.
2. `ports/` imports `domain/` only (for value types).
3. `usecases/` imports `domain/` + `ports/` only.
4. `adapters/primary/` and `adapters/secondary/` import `ports/` only.
5. Adapters NEVER import other adapters.
6. `composition-root` is the ONLY file that imports from adapters.
7. All relative imports MUST use `.js` extensions (NodeNext) for TypeScript adapters.

## Follow-on ADRs

This ADR is foundational; many later decisions assume it:

- [ADR-004](ADR-004-swarm-worktrees.md) — Parallel worktrees per adapter (relies on rule 5).
- [ADR-014](ADR-014-no-mock-module-di-deps.md) — Deps pattern over `mock.module()` (relies on ports as trait boundaries).
- [ADR-027](ADR-027-hexflo-coordination.md) — HexFlo swarm coordination (relies on adapter-scoped task assignment).
- [ADR-2026-04-06-1000](ADR-2026-04-06-1000-treesitter-architecture-enforcement.md) — Tree-sitter-based boundary checks.
- [ADR-054](ADR-054-adr-compliance-enforcement.md) — ADR compliance enforcement at the SOP layer.

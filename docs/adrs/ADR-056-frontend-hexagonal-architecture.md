# ADR-056: Frontend Hexagonal Architecture — Preventing UI Species Drift

**Status:** Accepted
**Date:** 2026-03-22
**Drivers:** Dashboard audit revealed 3 incompatible frontend applications (vanilla JS x2, SolidJS) with no shared state, duplicate WebSocket connections, and inconsistent styling — the exact class of problems hex architecture prevents on the backend.

## Context

Hex enforces strict hexagonal architecture on backend code: domain imports nothing, ports define contracts, adapters implement them, composition-root wires everything. These rules are checked by `hex analyze` and enforced by hooks.

**No equivalent enforcement exists for frontend code.** The dashboard evolved through multiple AI-assisted sessions without architectural constraints, resulting in:

| Symptom | Root Cause |
|---------|-----------|
| 3 HTML entry points | No "single composition root" rule |
| 2 styling systems (CSS vars + Tailwind) | No "one adapter per concern" rule |
| 3 chat implementations | No port interface for chat — each implementation was ad-hoc |
| Duplicate WebSocket connections | No "driven adapter singleton" rule |
| Inline `style={{}}` mixed with Tailwind | No boundary checking for styling |

This is **species drift** — the same problem that hexagonal architecture solves for backend code. When AI agents generate frontend code without structural constraints, each session produces a slightly different organism.

## Decision

Apply hexagonal architecture principles to frontend (SolidJS) code. The dashboard follows a **layered frontend architecture** that mirrors backend hex rules, enforced by the same `hex analyze` tooling.

### Layer Map

```
Frontend Hexagonal Layers
═══════════════════════════════════════════════════════════

  ┌─────────────────────────────────────────────────┐
  │  Domain (stores/)                                │
  │  Pure state + business logic. No DOM, no fetch.  │
  │  Signals, derived computations, state machines.  │
  │  IMPORTS: nothing outside domain                 │
  └─────────────────────────────────────────────────┘
                          │
  ┌─────────────────────────────────────────────────┐
  │  Ports (types/ or ports/)                        │
  │  TypeScript interfaces for external contracts.   │
  │  What data looks like, what actions exist.       │
  │  IMPORTS: domain types only                      │
  └─────────────────────────────────────────────────┘
                          │
  ┌─────────────────────────────────────────────────┐
  │  Adapters                                        │
  │                                                  │
  │  Primary (components/)                           │
  │    UI components that RENDER state.              │
  │    Import from stores (domain) + ports.          │
  │    NEVER import other adapters.                  │
  │    NEVER call fetch() or new WebSocket().        │
  │                                                  │
  │  Secondary (services/)                           │
  │    Data fetching, WebSocket, localStorage.       │
  │    Implement port interfaces.                    │
  │    Singleton per concern (one WS, one REST).     │
  │    NEVER import components.                      │
  └─────────────────────────────────────────────────┘
                          │
  ┌─────────────────────────────────────────────────┐
  │  Composition Root (app/App.tsx)                  │
  │  ONLY file that wires adapters to stores.        │
  │  Initializes connections, provides context.      │
  │  The ONLY place where services meet components.  │
  └─────────────────────────────────────────────────┘
```

### Rules (Enforceable)

| # | Rule | Rationale | Enforcement |
|---|------|-----------|-------------|
| F1 | **One entry point** — single `index.html` + single SPA | Prevents species drift | `hex analyze` checks for multiple HTML files |
| F2 | **Stores import nothing outside stores/** | State logic stays pure, testable | Import boundary check |
| F3 | **Components never call `fetch()` or `new WebSocket()`** | Data fetching belongs in services (secondary adapters) | AST grep for `fetch(` / `WebSocket(` in `components/` |
| F4 | **Components never import other adapter directories** | No cross-adapter coupling (same as backend rule 6) | Import boundary check |
| F5 | **One styling system** — Tailwind classes only, no inline `style={{}}` | Prevents dual-styling drift | AST grep for `style={` in TSX |
| F6 | **CSS custom properties only in `dashboard.css`** — components use Tailwind | Single source for theme tokens | File-scoped check |
| F7 | **Services are singletons** — one WebSocket connection, one REST client | Prevents duplicate connections | Module-level singleton pattern |
| F8 | **App.tsx is the only composition root** — wires services ↔ stores ↔ components | Same as backend rule 7 | Import graph analysis |
| F9 | **All colors from design system** — no hardcoded hex values in components | Prevents visual inconsistency | Grep for `#[0-9a-fA-F]{3,8}` in TSX |

### Directory Structure

```
hex-nexus/assets/src/
  app/
    App.tsx              # Composition root — ONLY file that wires everything
    index.tsx            # Entry point — renders App
    index.css            # Tailwind import
    dashboard.css        # Theme tokens (CSS custom properties)
  stores/                # Domain — pure state, signals, derived data
    router.ts            # Route state + breadcrumbs
    projects.ts          # Project state + CRUD
    chat.ts              # Chat state + message handling
    session.ts           # Session state + persistence
    connection.ts        # SpacetimeDB connection signals (NOT the adapter)
  services/              # Secondary adapters — data fetching, WebSocket
    spacetimedb.ts       # SpacetimeDB WebSocket connection (singleton)
    rest-client.ts       # Typed REST client (singleton)
    chat-ws.ts           # Chat WebSocket (singleton)
    local-storage.ts     # localStorage adapter
  components/            # Primary adapters — UI rendering
    chat/                # Chat-related components
    project/             # Project-related components
    layout/              # Layout components (sidebar, breadcrumbs, etc.)
    views/               # Page-level view components
    config/              # Configuration components
    swarm/               # Swarm/task components
  hooks/                 # Shared reactive utilities
    useTable.ts          # SpacetimeDB table → signal bridge
  types/                 # Port interfaces — shared TypeScript types
```

### Migration Path (from current state)

Current code violates several rules:
- `stores/chat.ts` contains `fetch()` and `new WebSocket()` (violates F2, should be in services/)
- `stores/connection.ts` creates WebSocket connections (violates F2)
- Multiple components use `style={{}}` (violates F5)
- Hardcoded colors in ADRBrowser, ControlPlane, etc. (violates F9)

Migration happens incrementally during P1-P5 of the dashboard redesign workplan:
1. **P0 (done)**: Consolidated to single entry point (F1 ✓), unified CSS (F6 ✓)
2. **P1**: Extract WebSocket/fetch from stores → services/ (F2, F3, F7)
3. **P2-P4**: Replace inline styles with Tailwind during component rewrites (F5, F9)
4. **P5**: Add `hex analyze` rules for frontend boundary checking (all rules)

### How `hex analyze` Enforces These Rules

Extend the existing tree-sitter analysis to check frontend code:

```
hex analyze . --frontend

Checks:
  [F1] Single entry point ............... ✓ (1 HTML file)
  [F2] Store purity ..................... ✗ stores/chat.ts imports fetch()
  [F3] Component fetch-free ............. ✓
  [F4] No cross-adapter imports ......... ✓
  [F5] No inline styles ................. ✗ 3 files with style={{}}
  [F6] CSS tokens in dashboard.css only . ✓
  [F7] Service singletons ............... ✓
  [F8] Single composition root .......... ✓
  [F9] No hardcoded colors .............. ✗ 5 files with hex literals
```

## Consequences

### Positive
- **Species drift becomes structurally impossible** — AI agents generating frontend code must follow the same boundary rules as backend code
- **Testability** — stores are pure (no I/O), services are mockable via ports, components are rendering-only
- **Single source of truth for styling** — one CSS file for tokens, Tailwind for application
- **Consistent evolution** — new features follow the same structure, no matter which agent or session creates them
- **Enforceable by tooling** — `hex analyze` catches violations before commit, same as backend

### Negative
- **More indirection** — components can't directly fetch data (must go through stores ← services)
- **Boilerplate** — service interfaces add types that simple `fetch()` calls didn't need
- **Migration cost** — current code needs refactoring (handled incrementally in redesign workplan)
- **Learning curve** — contributors must understand frontend hex layers (mitigated by `hex analyze` error messages)

### Risks
- **Over-abstraction** — frontend code is inherently more coupled to rendering than backend. Rules must be pragmatic (e.g., CSS-in-JS libraries would violate F5 but may be valid choices for other projects)
- **Tool support** — tree-sitter TypeScript/TSX analysis needs to be added to `hex analyze`

## References
- Dashboard UX Deep Audit: `docs/analysis/dashboard-ux-deep-audit-2026-03-22.md`
- Redesign Workplan: `docs/workplans/feat-dashboard-project-centric-redesign.json`
- ADR-052: AIIDE vision (design system, navigation model)
- ADR-046: SpacetimeDB single authority (state mutation rules)
- ADR-038: Vite dev / Axum prod (build pipeline)

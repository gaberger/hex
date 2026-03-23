# ADR-2603231000: Dashboard Reactive Context Fix — Eliminate Module-Level Computations

**Status:** Accepted
**Date:** 2026-03-23
**Drivers:** Dashboard SolidJS SPA crashes on load with `TypeError: workplans is not a function or its return value is not iterable` at WorkplanView.tsx:227. Console shows 15+ "computations created outside a `createRoot` or `render` will never be disposed" warnings. Root cause: reactive primitives (`createMemo`, `createEffect`, `createSignal`) created at module scope outside any reactive ownership context, violating Solid.js's reactive model.

## Context

The hex-nexus dashboard (SolidJS SPA at `hex-nexus/assets/src/`) has module-level reactive computations spread across four store files. These were introduced incrementally as features were added, with each store file creating signals/memos/effects at the top level during module evaluation.

### What's Broken

| File | Line | Primitive | Impact |
|------|------|-----------|--------|
| `stores/workplan.ts` | 49-52 | `createSignal` ×4 | Signal works but contributes to ownership warnings |
| `stores/projects.ts` | 22 | `createMemo` | Computation without owner — never disposed |
| `stores/router.ts` | 38, 42, 59 | `createSignal` + `createMemo` ×2 | Two ownerless computations |
| `stores/hexflo-monitor.ts` | 18 | `createEffect` (inside `startHexFloMonitor()`) | Effect runs without owner when called from `onMount` |
| `stores/connection.ts` | 48-49+ | `createSignal` ×6+ | Signals OK but no reactive root |

### The Crash

`WorkplanView.tsx:227` executes:
```typescript
const sortedWorkplans = createMemo(() => {
  return [...workplans()].sort((a, b) => { ... });
});
```

The `workplans` accessor is a module-level signal from `workplan.ts:49`. During Vite HMR cycles or when the reactive graph is in an inconsistent state (ownerless computations), the signal accessor can return `undefined` instead of `[]`, causing the spread operator to throw "not iterable".

### Why Module-Level Reactivity Is Wrong in Solid.js

Solid.js tracks reactive ownership via a tree: `render()` → components → computations. Computations (`createMemo`, `createEffect`) created outside this tree have no owner, meaning:
1. They are **never disposed** — memory leak on every HMR cycle
2. They may **not re-execute** when dependencies change (broken reactivity)
3. They **cannot be tracked** by parent computations (invisible to the graph)

`createSignal` is the exception — it's just a value container and doesn't need an owner. But even signals benefit from being scoped to a root for consistent lifecycle management.

### Relationship to ADR-056

ADR-056 (Frontend Hexagonal Architecture) defines the correct pattern: stores contain pure state, services handle I/O, and `App.tsx` is the composition root that wires everything. The current code violates this by having stores self-initialize their reactive graphs at import time rather than being initialized by the composition root.

## Decision

### 1. Wrap All Store Initialization in Explicit `createRoot` Blocks

Each store module exports an `init*()` function that creates its reactive primitives inside a `createRoot`. Signal accessors are exported as before (API-compatible), but the root provides proper ownership.

**Pattern:**
```typescript
// stores/workplan.ts — AFTER fix
import { createSignal, createRoot } from "solid-js";

let workplans: Accessor<WorkplanExecution[]>;
let setWorkplans: Setter<WorkplanExecution[]>;

export function initWorkplanStore() {
  createRoot(() => {
    const [_workplans, _setWorkplans] = createSignal<WorkplanExecution[]>([]);
    workplans = _workplans;
    setWorkplans = _setWorkplans;
  });
}

export { workplans };
```

### 2. Centralized Store Initialization in App.tsx

`App.tsx` calls all `init*()` functions in dependency order before rendering any components:

```typescript
// App.tsx — composition root
onMount(() => {
  initConnectionStore();   // must be first — other stores depend on signals
  initProjectStore();      // depends on connection signals
  initRouterStore();       // depends on project signals
  initWorkplanStore();     // independent
  initHexFloMonitor();     // depends on connection signals
  initConnections();       // starts WebSocket connections
});
```

### 3. Defensive Guards on Signal Accessors

All signal accessors used in spread/iteration contexts must guard against `undefined`:

```typescript
const sortedWorkplans = createMemo(() => {
  const list = workplans() ?? [];
  return [...list].sort((a, b) => { ... });
});
```

### 4. Move `createEffect` Out of `startHexFloMonitor`

The `createEffect` in `hexflo-monitor.ts:18` must be created inside a component or a `createRoot`, not inside a plain function called from `onMount`. Refactor to accept reactive dependencies as parameters.

## Consequences

**Positive:**
- Dashboard loads without crash — WorkplanView renders correctly
- Zero "computations outside createRoot" warnings in console
- Proper disposal on HMR — no memory leaks during Vite dev
- Aligns with ADR-056 composition root pattern
- Store initialization order is explicit and documented

**Negative:**
- Stores require an `init*()` call before use — importing alone isn't enough
- Slightly more boilerplate per store file
- Existing code that imports stores at module level needs to ensure init is called first

**Mitigations:**
- `App.tsx` composition root calls all init functions — single place to manage
- Runtime guard: accessor functions throw descriptive error if called before init
- Vite HMR preserves the reactive root across module reloads

## Implementation

| Phase | Description | Status |
|-------|------------|--------|
| P0 | Add `?? []` guard to WorkplanView.tsx:227 (immediate crash fix) | Pending |
| P1 | Refactor workplan.ts — wrap signals in createRoot, export init function | Pending |
| P2 | Refactor projects.ts — wrap createMemo in createRoot | Pending |
| P3 | Refactor router.ts — wrap createSignal + createMemo in createRoot | Pending |
| P4 | Refactor hexflo-monitor.ts — move createEffect into createRoot | Pending |
| P5 | Refactor connection.ts — wrap signals in createRoot | Pending |
| P6 | Update App.tsx — call all init functions in dependency order | Pending |
| P7 | Verify zero console warnings + successful HMR cycles | Pending |

## References

- ADR-056: Frontend hexagonal architecture (composition root pattern)
- ADR-066: Dashboard visibility overhaul (lists broken views)
- ADR-038: Vite dev / Axum prod (build pipeline)
- ADR-046: SpacetimeDB single authority (state mutation rules)
- SolidJS docs: [Reactive Roots](https://www.solidjs.com/docs/latest/api#createroot)

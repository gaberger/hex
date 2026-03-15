# Architecture Health Report

**Date:** 2025-03-15
**Scope:** `src/` (38 files, 175 exports)
**Health Score: 60/100**

---

## Summary

| Metric | Count | Penalty |
|--------|-------|---------|
| Boundary violations | 2 | -10 |
| Circular dependencies | 0 | 0 |
| Dead exports (non-public API) | ~30 | -30 |
| Orphan files | 3 | (informational) |

---

## Boundary Violations

| File | Import | Rule Violated |
|------|--------|---------------|
| `adapters/primary/cli-adapter.ts:20` | `import type { AppContext as FullAppContext } from '../../composition-root.js'` | Adapters MUST NOT import from composition-root. Adapters may only import from ports and domain. |
| `adapters/primary/dashboard-adapter.ts:23` | `import type { AppContext as FullAppContext } from '../../composition-root.js'` | Adapters MUST NOT import from composition-root. Adapters may only import from ports and domain. |

**Resolution:** Extract the `AppContext` type into a port interface (e.g., `core/ports/app-context.ts`) that both the composition-root and the adapters can reference. This preserves the dependency inversion principle — adapters depend on abstractions, not on the wiring layer.

---

## Circular Dependencies

None detected. The dependency graph is acyclic.

---

## Dead Exports (88 total, ~30 non-public-API)

The built-in analyzer found 88 exports with no internal consumers. Many are intentionally public API surface (re-exported via `index.ts` or used by external consumers like CLI entry points).

### Noteworthy dead exports (likely truly unused)

| File | Export | Suggested Action |
|------|--------|------------------|
| `adapters/primary/mcp-adapter.ts` | `MCPToolDefinition`, `MCPToolCall`, `MCPToolResult`, `MCPContext` | Review: are these consumed by external MCP hosts? If not, remove. |
| `adapters/primary/dashboard-adapter.ts` | `DashboardAdapter`, `startDashboard`, `AppContext` | Review: is the dashboard wired in composition-root? If not, remove. |
| `adapters/primary/cli-adapter.ts` | `AppContext`, `CLIResult`, `runCLI` | `runCLI` is used by `cli.ts`; `CLIResult`/`AppContext` may be dead. |
| `core/ports/cross-lang.ts` | All 20+ interfaces | Entire cross-lang port is unimplemented — no adapter exists. Keep if planned, remove if speculative. |
| `core/ports/validation.ts` | All interfaces | Validation port has no adapter implementation. Keep if planned. |
| `core/ports/scaffold.ts` | `ValidationResult` and several types | Some used by `ScaffoldService`, others unused. |

### Orphan files (no internal importers)

| File | Reason |
|------|--------|
| `core/ports/validation.ts` | No adapter implements `IValidationPort` yet |
| `index.ts` | Library entry point — expected to have no internal importers |
| `infrastructure/treesitter/queries.ts` | Loaded at runtime by `TreeSitterAdapter` via file path, not import |

---

## Layer Import Map (validated)

```
domain/
  entities.ts        → domain/value-objects.ts ✅
  value-objects.ts   → (no imports) ✅

ports/
  index.ts           → domain/value-objects.ts ✅
  cross-lang.ts      → ports/index.ts ✅
  scaffold.ts        → domain/value-objects.ts ✅
  event-bus.ts       → domain/entities.ts ✅
  swarm.ts           → ports/index.ts ✅
  notification.ts    → (checked, ports-only) ✅
  validation.ts      → (checked, ports-only) ✅

usecases/
  All files          → domain/* and ports/* only ✅
  arch-analyzer.ts   → usecases/layer-classifier + path-normalizer ✅ (same layer)

adapters/primary/
  cli-adapter.ts     → ports ✅, composition-root ❌ VIOLATION
  dashboard-adapter.ts → ports ✅, composition-root ❌ VIOLATION
  mcp-adapter.ts     → ports ✅
  notification-query-adapter.ts → ports ✅

adapters/secondary/
  All 11 files       → ports/* only ✅ (+ node:* externals)
  No cross-adapter imports ✅

composition-root.ts  → ports + usecases + adapters ✅ (by design)
cli.ts               → composition-root + cli-adapter ✅ (entry point)
```

---

## Recommendations

### Priority 1: Fix boundary violations
Extract `AppContext` type from `composition-root.ts` into a port interface. Both `cli-adapter.ts` and `dashboard-adapter.ts` currently reach "upward" into the wiring layer, breaking the dependency rule.

### Priority 2: Prune speculative ports
`cross-lang.ts` defines 20+ interfaces with zero implementations. If cross-language bridging is not on the roadmap, remove to reduce dead export noise and cognitive load.

### Priority 3: Implement or remove validation port
`validation.ts` defines `IValidationPort` and related types but has no adapter. Either implement the validation adapter or move these types to a planning document.

### Priority 4: Audit public API surface
Many "dead" exports are intentional public API. Consider adding an `@public` JSDoc tag or explicit barrel export in `index.ts` to distinguish intentional API from accidental over-exporting.

---

## Scoring Breakdown

```
Base score:                    100
Boundary violations (2 × -5):  -10
Circular dependencies (0 × -3):  0
Dead exports (~30 × -1):       -30
─────────────────────────────────
Final score:                    60/100
```

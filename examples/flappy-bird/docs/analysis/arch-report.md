# Architecture Health Report — Flappy Bird Example

**Date:** 2025-03-15
**Scope:** `examples/flappy-bird/src/`
**Health Score:** 100/100

---

## Summary

| Metric | Count |
|--------|-------|
| Files scanned | 10 |
| Boundary violations | 0 |
| Circular dependencies | 0 |
| Cross-adapter coupling | 0 |
| Dead exports | 0 |
| Orphan files | 0 |

The flappy-bird example fully adheres to hexagonal architecture rules. All dependency arrows point inward. Value types are owned by the domain layer and re-exported by ports for adapter convenience.

---

## Boundary Violations

None. Previously, `domain/game-state.ts` and `domain/physics.ts` imported types from `ports/index.js` (domain → ports violation). Fixed by extracting value types into `domain/types.ts` and having ports re-export them.

---

## Import Direction Map (all verified)

| From | To | Status |
|------|----|--------|
| `core/domain/game-state.ts` | `core/domain/types.js` | OK — domain → domain |
| `core/domain/physics.ts` | `core/domain/types.js` | OK — domain → domain |
| `core/ports/index.ts` | `core/domain/types.js` | OK — ports → domain |
| `core/usecases/game-engine.ts` | `core/ports/index.js` | OK — usecase → ports |
| `core/usecases/game-engine.ts` | `core/domain/game-state.js` | OK — usecase → domain |
| `adapters/primary/browser-input.ts` | `core/ports/index.js` | OK — adapter → port |
| `adapters/secondary/canvas-renderer.ts` | `core/ports/index.js` | OK — adapter → port |
| `adapters/secondary/browser-audio.ts` | `core/ports/index.js` | OK — adapter → port |
| `adapters/secondary/localstorage-adapter.ts` | `core/ports/index.js` | OK — adapter → port |
| `main.ts` (composition root) | adapters + usecases + ports | OK — composition root may import anything |

---

## Circular Dependencies

None detected.

---

## Cross-Adapter Coupling

None detected. Each adapter imports only from `core/ports/`.

---

## Dead Exports

No dead exports found. All exported symbols are consumed.

---

## Recommendations

1. **[LOW] Consider extracting physics constants.** `physics.ts` contains magic numbers (gravity, flap strength). These could live in a domain `constants.ts` for clarity, but this is cosmetic — the config is injected via `GameConfig` at runtime.

---

## Parent Project (hex framework) — Summary

The parent `hex` project reports:
- **Health Score:** 72/100
- **Dead exports:** 98 (many are public API surface — expected for a framework/library)
- **Orphan files:** 5 (`errors.ts`, `index.ts`, `validation.ts`, `queries.ts` — likely entry points or planned modules)
- **Boundary violations:** 0 detected by hex CLI
- **Circular dependencies:** 0 detected

The lower score is driven primarily by dead exports, which is normal for a library that exposes a broad public API consumed by external projects.

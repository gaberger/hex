# Research: Syntax-Based Approaches for Marking Intentional Exports

## Problem Statement

The hex dead-export analyzer (`ArchAnalyzer.findDeadFromSummaries()`) traces static `import` statements to find unused exports. It currently has hardcoded escape hatches:

- `ENTRY_POINTS` array: `index.ts`, `cli.ts`, `main.ts`, `composition-root.ts` (+ Go/Rust equivalents)
- `ENTRY_EXPORTS` set: `runCLI`, `startDashboard`, `createAppContext`, `main`, `init`
- `hasReExports()` heuristic: files where >50% of exports match import names

These miss three categories of legitimate exports:

1. **Dynamic `import()`** — composition-root.ts uses `await import(...)` for 8+ adapters (serialization, JSON schema, WASM bridge, FFI, service mesh, validation, hub-launcher, dashboard). The analyzer cannot see these as static edges.
2. **Public npm API surface** — `src/index.ts` re-exports port interfaces and domain types for npm consumers. These have no in-project importers.
3. **Composition root wiring** — Runtime DI binds adapters to ports. The adapter classes are exported but only consumed dynamically.

---

## Approach A: JSDoc/TSDoc Annotation

### How existing tools handle this

| Tool | Mechanism | Annotation |
|------|-----------|------------|
| **ESLint `no-unused-exports`** (eslint-plugin-import) | Ignores files matching `ignoreExports` glob array in config | No per-export annotation |
| **ts-prune** | `// ts-prune-ignore next` comment above an export | Line-level comment directive |
| **knip** | Config-level `entry`, `project`, `ignore` globs + `ignoreDependencies` | No per-export annotation; file-level entry point config |
| **unimported** | `.unimportedrc.json` with `ignorePatterns`, `ignoreUnused` arrays | Config-level allowlist |
| **TypeScript `@public`/`@internal`** | TSDoc standard tags, used by API Extractor (Microsoft) | `/** @public */` on declarations |

### Proposed syntax for hex

```typescript
/** @hex:public — Exported as npm public API */
export class ArchAnalyzer implements IArchAnalysisPort { ... }

/** @hex:dynamic — Consumed via dynamic import() in composition root */
export class SerializationAdapter { ... }

/** @hex:entry — CLI entry point, no static importers expected */
export function runCLI(): void { ... }
```

### Evaluation

| Criterion | Score | Notes |
|-----------|-------|-------|
| Developer ergonomics | Good | One comment per export; familiar JSDoc syntax |
| Analyzer complexity | Medium | Tree-sitter can extract JSDoc comments attached to exports; need a tree-sitter query for `(comment) @doc` preceding `(export_statement)` |
| False positive reduction | High | Directly addresses all 3 categories |
| Tree-sitter compatibility | Good | TypeScript grammar captures `comment` nodes; can match `@hex:` prefix via regex on comment text |

### Pros
- Zero runtime cost (comments stripped in build)
- Self-documenting: the annotation explains WHY the export exists
- Granular: per-export, not per-file
- Compatible with existing TSDoc tooling
- Tree-sitter can extract via `(comment) @doc` query + regex

### Cons
- Requires developer discipline to add annotations
- Need to define/document the tag vocabulary
- Not enforced by TypeScript compiler

---

## Approach B: TypeScript Inline Comment / Marker Type

### Option B1: Inline export comment

```typescript
export /* @hex:public */ class ArchAnalyzer { ... }
```

**Problem**: Tree-sitter parses this as a comment node inside the export statement, but it's fragile — formatters (prettier) may move or remove it. Not recommended.

### Option B2: Marker type

```typescript
// In a shared types file:
export type __HexPublicAPI = typeof ArchAnalyzer | typeof SummaryService;
export type __HexDynamicImport = typeof SerializationAdapter | typeof FFIAdapter;
```

### Evaluation

| Criterion | Score | Notes |
|-----------|-------|-------|
| Developer ergonomics | Poor (B1), Fair (B2) | B2 requires maintaining a separate list; easy to forget |
| Analyzer complexity | Easy (B2) | Just check if export name appears in a marker type's union |
| False positive reduction | High | Explicit allowlist |
| Tree-sitter compatibility | Poor (B1), Good (B2) | B2 is a normal type alias, easy to parse |

### Pros (B2)
- Type-checked by TypeScript: if you list `typeof FooAdapter` and `FooAdapter` doesn't exist, you get a compile error
- Single source of truth for public API declarations

### Cons (B2)
- Separate from the export site — easy to add an export and forget the marker
- Pollutes the type namespace with `__Hex` prefixed types
- Marker types are themselves dead exports (meta-problem)

---

## Approach C: Configuration File

### Proposed format

```jsonc
// .hex/exports.json
{
  "publicAPI": [
    "src/index.ts",           // All exports from this file are public API
    "src/core/ports/*.ts"     // Port interfaces are inherently public
  ],
  "dynamicImports": [
    "src/adapters/secondary/serialization-adapter.ts",
    "src/adapters/secondary/json-schema-adapter.ts",
    "src/adapters/secondary/wasm-bridge-adapter.ts",
    "src/adapters/secondary/ffi-adapter.ts",
    "src/adapters/secondary/service-mesh-adapter.ts",
    "src/adapters/secondary/validation-adapter.ts",
    "src/adapters/secondary/hub-launcher.ts",
    "src/adapters/primary/dashboard-adapter.ts"
  ],
  "entryPoints": [
    "src/cli.ts",
    "src/composition-root.ts"
  ],
  "allowedExports": {
    "src/core/domain/entities.ts": ["QualityScore", "FeedbackLoop", "TaskGraph"]
  }
}
```

### How knip does this (closest precedent)

knip uses `knip.json` with:
```json
{
  "entry": ["src/index.ts", "src/cli.ts"],
  "project": ["src/**/*.ts"],
  "ignore": ["**/*.test.ts"],
  "ignoreDependencies": ["@types/*"]
}
```

### Evaluation

| Criterion | Score | Notes |
|-----------|-------|-------|
| Developer ergonomics | Fair | Separate file to maintain; but can use globs for bulk rules |
| Analyzer complexity | Low | Simple: load JSON, check if file/export matches a pattern |
| False positive reduction | High | Glob patterns handle entire directories |
| Tree-sitter compatibility | N/A | No AST changes needed — config is read separately |

### Pros
- No source code modifications
- Glob patterns handle bulk cases (all ports, all entry points)
- Can be generated/validated by `hex setup`
- Familiar pattern (eslintrc, tsconfig, knip.json)

### Cons
- Separate file = can drift out of sync with actual exports
- Developers must remember to update it when adding new public exports
- No self-documentation at the export site

---

## Approach D: Convention-Based

### Option D1: File naming

```
src/core/ports/ast.port.ts        # .port.ts = always public
src/adapters/primary/cli.entry.ts  # .entry.ts = entry point
src/adapters/secondary/fs.adapter.ts  # Adapter naming already exists
```

### Option D2: Directory convention

```
src/public/          # Everything here is public API
src/core/ports/      # Ports are inherently public (already true in hex)
```

### Option D3: Hex layer rules (leverage existing architecture)

The hexagonal architecture already defines layers. The dead-export analyzer already calls `classifyLayer()`. We can define rules per layer:

| Layer | Default visibility | Rationale |
|-------|-------------------|-----------|
| `domain` | Internal (only consumed by ports/usecases) | Pure business logic |
| `ports` | **Public** (npm API surface) | Contracts are the public interface |
| `usecases` | Internal (consumed by adapters) | Application logic |
| `adapters/primary` | **Entry** (consumed dynamically or by composition root) | CLI, MCP, dashboard |
| `adapters/secondary` | **Entry** (consumed by composition root DI) | FS, Git, LLM, tree-sitter |
| `composition-root` | **Entry** (wires everything) | Already in ENTRY_POINTS |

This is the most natural fit for hex because it leverages the architecture that already exists.

### Evaluation

| Criterion | Score | Notes |
|-----------|-------|-------|
| Developer ergonomics | Excellent | Zero annotation work — layer membership implies visibility |
| Analyzer complexity | Low | `classifyLayer()` already exists; add visibility rules |
| False positive reduction | High for adapters/ports | May still need per-export escape hatch for edge cases |
| Tree-sitter compatibility | N/A | File path based, not AST based |

### Pros
- Zero developer effort — no annotations, no config files
- Consistent with hex philosophy: layers define boundaries
- Already partially implemented (ENTRY_POINTS, hasReExports)
- Enforces the mental model: "ports are contracts, adapters are wired at composition time"

### Cons
- Coarse-grained: all exports in a layer get the same treatment
- Cannot distinguish "this specific domain function is public API" vs "this is internal"
- Edge cases still need an escape valve

---

## Approach E: Lessons from Rust and Go

### Rust: Explicit visibility scoping

```rust
pub fn public_api() { }              // Public to all crates
pub(crate) fn crate_internal() { }   // Public within this crate only
pub(super) fn parent_module() { }    // Public to parent module only
fn private() { }                     // Module-private (default)
```

Rust's `pub(crate)` is the closest analog to "exported but not part of npm public API." TypeScript has no equivalent — `export` is binary (exported or not).

### Go: Convention-based (uppercase = public)

```go
func PublicAPI() { }   // Uppercase = exported from package
func internal() { }    // Lowercase = package-private
```

Go's approach is purely convention-based and enforced by the compiler. TypeScript could adopt a similar convention but cannot enforce it at the language level.

### What hex could borrow

Neither language has the "dead export" problem because their module systems are richer:
- Rust: `pub(crate)` lets you export for internal use without making it public API
- Go: Package boundaries naturally scope visibility

TypeScript's `export` is flat — once exported, it's visible to everything. The hex equivalent would be:

```typescript
// Hypothetical: hex could define visibility levels in a comment
/** @hex:visibility crate */   // Equivalent to pub(crate) — internal to hex
/** @hex:visibility public */  // Equivalent to pub — npm API surface
/** @hex:visibility entry */   // Entry point — consumed dynamically
```

But this is just Approach A with Rust-inspired vocabulary.

---

## Recommendation for hex

### Primary: Approach D3 (Layer-Based Convention) + Approach A (JSDoc Annotations) as escape hatch

This is a **two-tier** system:

#### Tier 1: Layer-based defaults (zero effort, covers ~90% of cases)

Modify `findDeadFromSummaries()` to apply layer-aware visibility rules:

```typescript
function shouldSkipDeadExportCheck(filePath: string): boolean {
  const layer = classifyLayer(filePath);

  // Ports are contracts — they ARE the public API. Never flag as dead.
  if (layer === 'ports') return true;

  // Adapters are wired via composition root (often dynamic import).
  // Their class/function exports are consumed at runtime, not via static imports.
  if (layer === 'adapters/primary' || layer === 'adapters/secondary') return true;

  // Entry points already handled
  if (isEntryPoint(filePath)) return true;

  // Re-export barrel files already handled
  // Domain and usecases: DO check for dead exports (these should have static consumers)
  return false;
}
```

This immediately eliminates false positives for:
- All port interfaces (they're contracts, consumed by adapters implementing them)
- All adapter exports (wired by composition root via dynamic import)
- Entry points (already handled)

Dead export checking remains strict for:
- Domain layer (pure functions/types should have static consumers in ports/usecases)
- Use cases (should be consumed by adapters or composition root)

#### Tier 2: `@hex:public` JSDoc annotation (per-export escape hatch)

For edge cases where a specific domain or use-case export is intentionally public but has no in-project static consumer:

```typescript
/** @hex:public — Re-exported from index.ts for npm consumers */
export class QualityScore { ... }
```

The analyzer extracts this via tree-sitter:

```
;; Tree-sitter query to find @hex: annotations
(export_statement
  (comment) @doc
  (#match? @doc "@hex:(public|dynamic|entry)")
) @export
```

Note: Tree-sitter's TypeScript grammar attaches preceding comments to the next statement. The query needs to check for `comment` nodes that are siblings preceding an `export_statement`. The exact query depends on the grammar version, but a practical implementation would:

1. For each export found as dead, check if the preceding sibling node is a comment containing `@hex:public`
2. If so, skip it

#### Implementation cost

| Component | Effort | Files to change |
|-----------|--------|-----------------|
| Layer-based skip logic | ~20 lines | `arch-analyzer.ts` (add `shouldSkipDeadExportCheck`) |
| JSDoc annotation parsing | ~30 lines | `arch-analyzer.ts` (check preceding comment in L1 summary) |
| L1 summary enhancement | ~15 lines | Tree-sitter adapter needs to capture leading comments on exports |
| Documentation | ~1 page | Add to CLAUDE.md rules and agent instructions |

#### Why not config file (Approach C)?

Config files drift. In a framework that gets installed into target projects via `hex setup`, you'd need to generate/maintain `.hex/exports.json` per target project. Layer conventions travel with the architecture itself — no extra config.

#### Why not marker types (Approach B)?

Marker types are themselves dead exports (meta-problem). They pollute the type namespace and are disconnected from the export site.

#### Why not pure convention (Approach D1/D2)?

File naming conventions (`.public.ts`) would require renaming existing files and changing import paths across the codebase. Too disruptive for the benefit.

---

## Summary

| Approach | Ergonomics | Complexity | FP Reduction | Tree-sitter | Recommended |
|----------|-----------|------------|--------------|-------------|-------------|
| A: JSDoc `@hex:public` | Good | Medium | High | Good | Yes (Tier 2) |
| B1: Inline comment | Poor | High | Medium | Poor | No |
| B2: Marker type | Fair | Easy | High | Good | No |
| C: Config file | Fair | Low | High | N/A | No |
| D1: File naming | Poor | Low | Medium | N/A | No |
| D2: Directory | Fair | Low | Medium | N/A | No |
| **D3: Layer rules** | **Excellent** | **Low** | **High** | **N/A** | **Yes (Tier 1)** |
| E: Rust/Go inspired | Good | Medium | High | Good | Informs design |

**Final recommendation**: Layer-based defaults (D3) as the primary mechanism, with `@hex:public` JSDoc annotations (A) as the per-export escape hatch. This gives zero-effort coverage for ~90% of false positives and a clean, self-documenting escape valve for the remaining 10%.

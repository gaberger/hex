# ADR-2604061000: Tree-Sitter as Architecture Enforcement Engine

**Status:** Accepted
**Date:** 2026-04-06
**Drivers:** ADR-001 mandates hexagonal boundary rules but specifies only "lint rules" without defining the mechanism. ADR-002 establishes tree-sitter for token-efficient summaries but does not cover its role in enforcement. This ADR closes the gap — tree-sitter is the engine that powers `hex analyze`.
**Supersedes:** None (extends ADR-001 and ADR-002)

## Context

hex enforces hexagonal architecture (ADR-001) across TypeScript, Go, and Rust codebases. Traditional enforcement approaches have significant drawbacks:

- **Compiler-based**: Requires building the project (slow, language-specific toolchains, fails on incomplete code)
- **Regex/grep-based**: Fragile, can't distinguish `import` in a string literal from an actual import statement
- **LSP-based**: Requires a running language server per language, heavy resource usage, not embeddable in a CLI

Tree-sitter solves all three problems. It parses source code into a concrete syntax tree **without compiling**, works identically across languages via grammar plugins, runs in milliseconds, and is embeddable as a native Rust library.

hex already uses tree-sitter for L0-L3 AST summaries (ADR-002). This ADR formalizes that the **same parsing infrastructure** also powers all architecture enforcement rules.

## Decision

Tree-sitter is the **single enforcement engine** for hexagonal architecture analysis in `hex analyze`. All boundary checking, dead code detection, cycle detection, and layer classification derive from tree-sitter AST extraction — no compilation, no external toolchains, no language servers required.

### Enforcement Pipeline

```
Source files → Tree-sitter parse → Import/Export extraction → Layer classification → Rule evaluation → Violations
```

### Supported Languages

| Language | Grammar Crate | File Extensions | Layer Conventions |
|----------|--------------|-----------------|-------------------|
| TypeScript | `tree-sitter-typescript` | `.ts`, `.tsx`, `.js`, `.jsx` | `domain/`, `ports/`, `adapters/primary/`, `adapters/secondary/`, `usecases/` |
| Go | `tree-sitter-go` | `.go` | `internal/domain/`, `internal/ports/`, `cmd/` (primary), `pkg/` (ports), `internal/` (usecases) |
| Rust | `tree-sitter-rust` | `.rs` | Module paths + `Cargo.toml` workspace member classification |

### Extraction Capabilities

**Imports** — resolved per-language:
- TypeScript: `import { X } from './path'`, `import * as X`, `import('./path')`, `export { X } from`
- Go: `import "pkg"`, `import ( "pkg1"; "pkg2" )`, side-effect imports `import _ "pkg"`
- Rust: `use crate::module::Item`, `use super::Item`, `mod item`

**Exports** — resolved per-language:
- TypeScript: `export function`, `export class`, `export interface`, `export type`, `export const`, `export default`, `export { X }`
- Go: Capitalized identifiers (`func Handler`, `type Config struct`) — Go's visibility convention
- Rust: `pub fn`, `pub struct`, `pub enum`, `pub trait`, `pub mod`

**Annotations**: `@hex:public` marker detected across all languages to explicitly mark internal APIs as public.

### Enforcement Rules

All rules from ADR-001 are implemented via tree-sitter import graph analysis:

| Rule ID | Description | Tree-Sitter Mechanism |
|---------|-------------|----------------------|
| HEX-001 | Domain imports only domain | Extract imports from domain/ files → verify all resolve to domain/ |
| HEX-002 | Ports import only domain | Extract imports from ports/ files → verify targets are domain/ only |
| HEX-003 | Adapters import only ports | Extract imports from adapters/ → verify targets are ports/ (not other adapters, not domain directly) |
| HEX-004 | No cross-adapter imports | Extract imports from each adapter → verify none resolve to another adapter |
| HEX-005 | No circular dependencies | Build directed import graph → detect cycles via DFS |
| HEX-006 | Dead export detection | Scan all exports across all files → trace consumers → report exports with zero consumers |
| HEX-007 | Composition root exclusivity | Verify only `composition-root` (or `main.go` / `main.rs`) imports from adapters |

### Progressive Detail Levels (shared with ADR-002)

The same tree-sitter parse produces both enforcement data and agent context:

| Level | Tokens/File | Enforcement Use | Agent Use |
|-------|------------|-----------------|-----------|
| L0 | ~5 | File list for boundary scan scope | Project orientation |
| L1 | ~50 | Import/export graph for rule evaluation | Dependency mapping |
| L2 | ~200 | Signature analysis for interface compliance | Related file context |
| L3 | ~2000 | Full source (not used for enforcement) | File under active edit |

### Implementation Location

```
hex-nexus/src/analysis/
  treesitter_adapter.rs    # Parse → extract imports/exports per language
  boundary_checker.rs      # HEX-001 through HEX-004 rule evaluation
  cycle_detector.rs        # HEX-005 directed graph cycle detection
  dead_export_finder.rs    # HEX-006 unused export scanning
  layer_classifier.rs      # Map file paths → hex layers per language convention
  frontend_checker.rs      # ADR-056 frontend-specific rules (F1-F9)
  analyzer.rs              # Orchestrator: run all checks, aggregate violations
  domain.rs                # Language enum, Violation types, AnalysisResult

hex-nexus/src/analysis/
  fingerprint_extractor.rs # Project fingerprinting (framework, language, style)

config/treesitter.json     # L1/L2 query patterns per language
```

## Consequences

**Positive:**
- Single mechanism for enforcement across TypeScript, Go, and Rust — no per-language toolchain dependency
- Analysis runs in milliseconds (no compilation step), works on incomplete/broken code
- Same parse serves both enforcement (this ADR) and agent context (ADR-002) — no duplicate work
- Runs offline — no daemon, no SpacetimeDB, no network required for `hex analyze`
- Adding a new language requires only a tree-sitter grammar crate + layer convention mapping

**Negative:**
- Tree-sitter extracts syntactic structure, not semantic types — cannot verify Go interface compliance (implicit interfaces)
- Import path resolution is syntactic: Go module paths (`github.com/org/repo/internal/domain`) require `go.mod` parsing to map to local files
- Cannot detect runtime violations (e.g., reflection-based imports, dynamic `require()`)

**Mitigations:**
- Go module path resolution: parse `go.mod` to build module→filesystem mapping (planned, not yet implemented)
- Interface compliance: planned as a separate pass using type information extracted from tree-sitter struct/interface nodes
- Runtime violations: out of scope — hex enforces static architecture; runtime behavior is the domain of tests

## References

- ADR-001: Hexagonal Architecture as Foundational Pattern (defines the rules)
- ADR-002: Tree-Sitter for Token-Efficient LLM Communication (defines the parsing levels)
- ADR-003: Multi-Language Support — TypeScript, Go, Rust (defines supported languages)
- ADR-034: Migrate Hex Analyzer from TypeScript to Rust (native tree-sitter bindings)
- ADR-054: ADR Compliance Enforcement (extends rule set beyond hex boundary rules)
- ADR-2603283000: Rust Workspace Boundary Analysis (Rust-specific layer classification)

# ADR-003: Multi-Language Support — TypeScript, Go, Rust

**Status:** Accepted
## Date: 2026-03-15

## Context

hex needs to support code generation, linting, and testing across multiple languages. The framework targets LLM-driven development where compile-time feedback is the primary guardrail against defects. We need languages with strong static typing, mature tree-sitter grammars, and fast build toolchains.

## Decision

We support three languages as first-class targets: **TypeScript**, **Go**, and **Rust**.

### Rationale per Language

| Criteria | TypeScript | Go | Rust |
|----------|-----------|-----|------|
| **Why included** | Largest LLM training corpus; dominant in AI tooling | Fast compile; simple type system; excellent concurrency | Strongest type system; memory safety without GC |
| **Tree-sitter grammar** | `tree-sitter-typescript` (mature, WASM-ready) | `tree-sitter-go` (stable, well-maintained) | `tree-sitter-rust` (stable, covers full syntax) |
| **Build chain** | tsc (type check) + esbuild (bundle) | `go build` (single binary) | `cargo build` (with incremental compilation) |
| **Linter** | ESLint + tsc strict mode | golangci-lint (aggregates 50+ linters) | clippy (compiler-integrated) |
| **Test runner** | vitest / jest | `go test` (built-in) | `cargo test` (built-in) |
| **Compile speed** | ~1s (esbuild) | ~2s | ~5-30s (mitigated by incremental builds) |

### Build Toolchain Configuration

Each language maps to a concrete `IBuildPort` adapter:

- **TypeScript**: `tsc --noEmit` for type checking, `esbuild` for bundling, `vitest` for tests
- **Go**: `go build ./...`, `golangci-lint run`, `go test ./...`
- **Rust**: `cargo check` (fast type check), `cargo clippy`, `cargo test`

### Tree-sitter Grammar Availability

All three grammars support the full AST extraction pipeline needed for L0-L3 summaries. Each grammar can extract exports, imports, function signatures, and type definitions — the four structural elements required by `ASTSummary`.

## Consequences

### Positive

- Covers ~80% of production backend/systems codebases
- All three have compile-time error detection, enabling the fast feedback loop (ADR-005)
- Tree-sitter grammars are mature and WASM-compatible for in-browser use
- Port interfaces can be generated across languages from a single TypeScript definition

### Negative

- Rust compile times (5-30s) slow the feedback loop; mitigated by `cargo check` (~2s) for type-only validation
- Three languages triple the adapter surface area for `IBuildPort` and `IASTPort`
- LLM accuracy varies by language — Rust generation requires more correction iterations

### Risks

- Tree-sitter grammar updates may lag behind language releases (e.g., new Rust syntax)
- Maintaining port interface parity across three languages adds ongoing cost

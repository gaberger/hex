# Tree-Sitter AST Summary Format Specification

## Overview

This document defines the canonical output format for tree-sitter AST summaries used by hex. The format is designed for maximum token efficiency while preserving the structural information an LLM needs to generate compatible code across TypeScript, Go, and Rust.

---

## Summary Levels

### L0 -- Index

Minimal file identification. Used for project-wide file listings.

```
FILE: <relative-path>
LANG: <typescript|go|rust>
LINES: <integer>
```

**Token budget:** ~5 tokens per file.

### L1 -- Skeleton

Export names, import sources, and external dependencies. No type signatures.

```
FILE: <relative-path>
LANG: <typescript|go|rust>
EXPORTS:
  <kind> <name>
  ...
IMPORTS: [<name>, ...] from <source>
  ...
DEPS: <pkg>, ...
LINES: <integer>
```

**Field definitions:**

| Field | Required | Description |
|-------|----------|-------------|
| `FILE` | yes | Relative path from project root |
| `LANG` | yes | One of `typescript`, `go`, `rust` |
| `EXPORTS` | yes | One line per exported symbol: `<kind> <name>` |
| `IMPORTS` | yes | Grouped by source module |
| `DEPS` | no | External (non-project) package names |
| `LINES` | yes | Total source line count |

**Valid `kind` values by language:**

| TypeScript | Go | Rust |
|------------|-----|------|
| `function` | `func` | `fn` |
| `class` | `struct` | `struct` |
| `interface` | `interface` | `trait` |
| `type` | `type` | `type` |
| `const` | `const` | `const` |
| `enum` | -- | `enum` |

**Token budget:** ~30-60 tokens per file.

### L2 -- Signatures

Full type signatures including parameters, return types, and struct/interface members. This is the primary level for cross-file reasoning.

```
FILE: <relative-path>
LANG: <typescript|go|rust>
EXPORTS:
  <kind> <name>
    <visibility> <member-signature>
    ...
IMPORTS: [<name>, ...] from <source>
  ...
DEPS: <pkg>, ...
LINES: <integer>
TOKENS: <integer>
```

**Signature format by language:**

TypeScript:
```
  interface IExample
    + methodName(param: Type, param2: Type): ReturnType
  type Alias = UnionMember | UnionMember
  class Example implements IPort
    + constructor(dep: Type)
    + async method(p: T): Promise<R>
    - privateMethod(): void
```

Go:
```
  interface Example
    MethodName(param Type, param2 Type) (ReturnType, error)
  struct Example
    + FieldName Type
    + MethodName(param Type) ReturnType
  func StandaloneFunc(p Type) ReturnType
```

Rust:
```
  trait Example
    fn method_name(&self, param: Type) -> ReturnType
  struct Example
    + field_name: Type
  impl Example
    + fn method_name(&self, param: Type) -> Result<R, E>
  enum Example
    VariantA(Type)
    VariantB { field: Type }
```

**Visibility markers:**

| Marker | Meaning |
|--------|---------|
| `+` | Public / exported |
| `-` | Private / unexported |
| `#` | Protected (TypeScript only) |

**Token budget:** ~100-300 tokens per file.

### L3 -- Full Source

Complete source code. Only loaded for the file currently being edited.

```
FILE: <relative-path>
LANG: <typescript|go|rust>
LINES: <integer>
TOKENS: <integer>
---
<full source code>
```

**Token budget:** ~500-5000 tokens per file (depends on file size).

---

## Token Estimates by Level and Language

| Level | TypeScript | Go | Rust | Description |
|-------|-----------|-----|------|-------------|
| L0 | 5 | 5 | 5 | Path + lang + lines |
| L1 | 40 | 35 | 45 | + export names, imports |
| L2 | 200 | 180 | 220 | + full signatures |
| L3 | 2000 | 1800 | 2200 | Full source (500-line file) |

Rust L2 summaries are slightly larger due to lifetime annotations, `Result` types, and `impl` blocks being separate from `struct` definitions.

---

## Structural Diff Format

Used by `IASTPort.diffStructural()` to compare two summaries of the same file.

```
DIFF: <relative-path>
LANG: <typescript|go|rust>
ADDED:
  <kind> <name>
    <signature>
REMOVED:
  <kind> <name>
    <signature>
MODIFIED:
  <kind> <name>
    - <old-signature>
    + <new-signature>
```

**Rules:**
- Only exported symbols appear in diffs.
- Signature changes (parameter types, return types) count as MODIFIED.
- Renamed symbols appear as REMOVED + ADDED.
- Import changes are not tracked in structural diffs (they are derived from exports).
- If a section is empty, omit it entirely.

**Example:**

```
DIFF: src/core/ports/index.ts
LANG: typescript
ADDED:
  interface IMetricsPort
    + recordEvent(name: string, data: Record<string, unknown>): Promise<void>
MODIFIED:
  interface IBuildPort
    - test(project: Project, suite: TestSuite): Promise<TestResult>
    + test(project: Project, suite: TestSuite, timeout?: number): Promise<TestResult>
```

---

## Parser Configuration Requirements

Each language requires a tree-sitter grammar and a set of query patterns per summary level. See `config/treesitter.json` for the full configuration.

### Grammar Packages

| Language | npm Package | Grammar Source |
|----------|-------------|----------------|
| TypeScript | `tree-sitter-typescript` | `typescript/src` |
| Go | `tree-sitter-go` | `src` |
| Rust | `tree-sitter-rust` | `src` |

### Query Pattern Requirements

Each summary level uses tree-sitter S-expression queries to extract nodes:

- **L0**: No queries needed (metadata only from parser).
- **L1**: Capture `export_statement`, `import_declaration`, top-level `function_declaration`, `class_declaration`, `interface_declaration`, `type_alias_declaration`.
- **L2**: L1 captures + `method_signature`, `method_definition`, `property_signature`, `public_field_definition`, parameter lists, type annotations, return types.
- **L3**: Full source pass-through (no queries).

### Node Type Mapping

| Concept | TypeScript Node | Go Node | Rust Node |
|---------|----------------|---------|-----------|
| Function | `function_declaration` | `function_declaration` | `function_item` |
| Class/Struct | `class_declaration` | `type_declaration` (struct) | `struct_item` |
| Interface/Trait | `interface_declaration` | `type_declaration` (interface) | `trait_item` |
| Type Alias | `type_alias_declaration` | `type_declaration` (alias) | `type_item` |
| Method | `method_definition` | `method_declaration` | `impl_item` > `function_item` |
| Import | `import_statement` | `import_declaration` | `use_declaration` |
| Export | `export_statement` | (capitalized name) | `pub` visibility |
| Enum | `enum_declaration` | -- | `enum_item` |
| Const | `lexical_declaration` (const) | `const_declaration` | `const_item` |

---

## Encoding Rules

1. **UTF-8 only.** All summaries are plain UTF-8 text.
2. **Two-space indentation** for nested members under exports.
3. **No trailing whitespace.**
4. **No blank lines** within a section; one blank line between sections is optional.
5. **Names are unquoted** unless they contain special characters (rare).
6. **Generic type parameters** are included in signatures: `Promise<T>`, `Result<T, E>`, `chan T`.
7. **Async markers** are preserved: `async` in TypeScript, implicit in Go (goroutine patterns are not captured), `async` in Rust.
8. **Lifetime annotations** in Rust are included when present: `&'a str`.
9. **Receiver types** in Go and Rust are included: `(s *Server)`, `(&self)`.

# ADR-002: Tree-Sitter for Token-Efficient LLM Communication

**Status:** Accepted
## Date: 2026-03-15

## Context

Sending raw source to LLMs wastes 80-90% of tokens on implementation details, whitespace, and comments. Agents need structural understanding (what exists, what types, what contracts) far more often than full source.

## Decision

Use tree-sitter to extract progressive AST summaries at four levels:

| Level | ~Tokens/File | Contents |
|-------|-------------|----------|
| L0 | 5 | filename, language, line count |
| L1 | 50 | exports, imports, dependency list |
| L2 | 200 | full type signatures with params and returns |
| L3 | 2000 | complete source (only when editing) |

Agents load L1 for orientation, L2 for related files, L3 only for the file under edit.

## Consequences

- **Positive**: 10x token reduction for context loading
- **Positive**: Agents can "see" entire project structure in ~500 tokens
- **Positive**: Structural diffs detect unintended API changes
- **Negative**: Requires tree-sitter grammars per language (TS, Go, Rust all supported)
- **Negative**: Comments and documentation are stripped — agents lose narrative context

## Mitigation

L2 summaries include JSDoc/GoDoc first-line descriptions when present. A separate `CONTEXT.md` per adapter provides narrative that the AST cannot capture.

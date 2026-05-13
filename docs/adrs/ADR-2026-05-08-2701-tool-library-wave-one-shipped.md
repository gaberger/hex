# ADR-2026-05-08-2701 — tool-library-wave-one-shipped

Status: **Accepted** (shipped 2026-05; cargo_check / repo_grep / adr_draft all live per memory `project_typed_tool_sop_proven`; title literally says "shipped")
Date: 2026-05-09

## Context

Wave one of the typed tool library comprises three tools implemented in Rust under `hex-nexus/src/tools/`:

1. **`cargo_check.rs`** — Wraps `cargo check --workspace --message-format=json` with a 60-second timeout. Returns structured compiler errors and warnings (level, rendered message, file, line) capped at 30 each. Accepts optional `crate` parameter to narrow the check to a single crate (e.g. `hex-nexus`, `hex-cli`) and an optional `release` boolean to select the release profile. Used as a deterministic oracle in Phase 4 (verification) to confirm Rust code changes compile before an artifact is considered done.

2. **`repo_grep.rs`** — Wraps ripgrep with a 5-second timeout. Searches the hex repository for a regex pattern and returns file:line:content matches capped at 50 by default (hard cap 200). Accepts required `pattern` string (ripgrep regex syntax), optional `glob` to restrict file paths (e.g. `*.rs`, `docs/adrs/*.md`), and optional `max_matches` integer. Used in the GROUND phase to surface concrete repository facts before reasoning. Sets a `truncated` flag when results are capped, signaling the model to narrow the query if needed.

3. **`adr_draft.rs`** — Validates and writes a proposed_action(kind=file_write) row to SpacetimeDB for a new Architecture Decision Record. Requires `id` (10+ digit timestamp form), `title` (1-80 chars, kebab-case-friendly), `status` (enum: proposed|accepted|superseded), and `body` (200-50000 bytes). Validates the body contains the three mandatory sections: `## Context`, `## Decision`, and `## Consequences`. The digital-twin executor materializes the file at `docs/adrs/ADR-<id>-<slug>.md` after approval. This is the cto persona's primary artifact-production tool.

All three tools implement the `Tool` async trait with `name`, `description`, `input_schema` (JSON Schema), and `execute` methods. Each returns a `ToolResult` with ok/error status, elapsed milliseconds, and an optional `truncated` flag for bounded result sets.

## Decision

We ship wave one consisting of `cargo_check`, `repo_grep`, and `adr_draft` as the foundational tool library for hex-nexus orchestration. These three tools provide the minimal viable coverage for grounding, verification, and artifact production in the SOP (ADR-2026-05-08-2500) workflow: GROUND → REASON → DECIDE → ACT.

- `repo_grep` supports the GROUND phase.
- `cargo_check` supports the DECIDE/ACT verification gate.
- `adr_draft` supports the ACT phase for producing architecture decisions.

## Consequences

**Positive:**
- CTO persona can ground decisions in real repository facts via `repo_grep`, verify Rust changes compile via `cargo_check`, and produce ADRs via `adr_draft`.
- Typed tool library pattern is proven: each tool exposes JSON Schema, validates inputs, returns structured results, and respects bounded execution (timeout, cap).
- The three tools cover the critical path for the typed-tool-library-and-sop-execution architecture.

**Negative:**
- Wave one does not include `repo_read`, `cargo_test`, `escalate_to_operator`, or higher-level composition tools (e.g. `git_commit`, `pr_open`). These will be added in wave two.
- No direct file-write tool for arbitrary code changes; `adr_draft` only writes ADRs. Wave two will add a general `file_write` or `apply_diff` tool for code modifications.

**Neutral:**
- Tool registration in `hex-nexus/src/tools/mod.rs` is manual. Future waves may introduce a procedural macro or discovery pattern.
- Error handling currently returns string messages; wave two may introduce typed error enums for richer LLM reasoning.
# ADR-2603241226: Structured CLI Table Output via `tabled`

**Status:** Accepted
**Date:** 2026-03-24
**Drivers:** CLI commands (`hex adr list`, `hex task list`, `hex agent list`, `hex status`, etc.) use hand-crafted `println!` with manual padding and Unicode box-drawing characters. This is fragile, hard to maintain, and produces inconsistent formatting across commands.

## Context

hex-cli has 9+ files with manual table formatting using `println!` with hardcoded separators (`━`, `─`, `|`). Each command reinvents column alignment, truncation, and color. This leads to:
- Inconsistent column widths across commands
- No automatic terminal-width adaptation
- Duplicated formatting logic
- Difficult to add new columns or change layout

The Rust ecosystem offers several table formatting crates:
- **tabled** (2k+ stars): `#[derive(Tabled)]` on structs — zero boilerplate, supports colors, themes, column width limits, and custom formatting
- **comfy-table**: fluent API, dynamic width — more control but more verbose
- **prettytable-rs**: macro-based `row![]` — older, less maintained

## Decision

We will add `tabled` as a dependency and create a shared `hex-cli/src/fmt.rs` module that provides:

1. A `HexTable` wrapper that applies consistent hex branding (colors, borders) to any `tabled::Table`
2. Standard themes: `compact` (no borders, for piping), `default` (rounded borders), `status` (colored headers)
3. Helper functions for common patterns: truncation, colored status badges, relative timestamps
4. All CLI commands will migrate from manual `println!` tables to `#[derive(Tabled)]` structs rendered through `HexTable`

The `colored` crate (already a dependency) will be used for cell-level coloring.

## Consequences

**Positive:**
- Consistent table formatting across all CLI commands
- Automatic terminal-width handling
- Trivial to add/remove/reorder columns
- `--json` output already exists separately; tables are for human consumption only
- Derive macro means new commands get tables for free

**Negative:**
- New dependency (+~50KB compile)
- Migration touches 9 files

**Mitigations:**
- Migration is mechanical — replace `println!` blocks with derive + render
- `tabled` is well-maintained with 0 unsafe code

## Implementation

| Phase | Description | Status |
|-------|------------|--------|
| P1 | Add `tabled` dep + `fmt.rs` module with HexTable wrapper | Pending |
| P2 | Migrate `hex adr list/status/search` | Pending |
| P3 | Migrate `hex agent list/info`, `hex task list`, `hex plan list` | Pending |
| P4 | Migrate `hex status`, `hex inference list`, `hex test trends` | Pending |

## References

- [tabled crate](https://crates.io/crates/tabled)
- ADR-2603241126: TUI CLI Surrogate (ratatui for interactive mode — this ADR covers non-interactive CLI output)

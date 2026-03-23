# ADR-2603221500: Timestamp-Based ADR Numbering (YYMMDDHHMM)

**Status:** Accepted
**Date:** 2026-03-22
**Drivers:** Duplicate ADR-059 collision (two files with same sequential number); multi-agent concurrent ADR creation makes sequential `max+1` unreliable
**Supersedes:** None (extends ADR-012 lifecycle tracking)

## Context

ADR numbering currently uses sequential three-digit IDs (`ADR-001` through `ADR-066`). The next number is determined by scanning the `docs/adrs/` directory for the highest existing number and adding 1. This approach has two problems:

1. **Race condition**: When two agents (or a human and an agent) create ADRs concurrently, both scan the directory, both compute the same `max + 1`, and both write a file with the same number. This already happened — `ADR-059-canonical-project-identity.md` and `ADR-2603221522-embedded-asset-bundle.md` both exist.

2. **Offline/disconnected creation**: The SpacetimeDB reservation mechanism (`POST /api/adr/reserve`) mitigates this when nexus is running, but fails silently when nexus is offline — falling back to the race-prone filesystem scan.

3. **Multi-project coordination**: As hex manages multiple target projects, ADR numbers from different repos could collide when cross-referenced.

Timestamp-based numbering eliminates these problems by using the creation time as the identifier, making collisions virtually impossible without any coordination mechanism.

## Decision

**All new ADRs SHALL use `YYMMDDHHMM` format for their numeric identifier.**

### Format Specification

```
ADR-YYMMDDHHMM-kebab-slug.md

Where:
  YY   = two-digit year (26 = 2026)
  MM   = two-digit month (01–12)
  DD   = two-digit day (01–31)
  HH   = two-digit hour in 24h format (00–23)
  MM   = two-digit minute (00–59)

Example: ADR-2603221500-timestamp-adr-numbering.md
         → Created 2026-03-22 at 15:00
```

### Backward Compatibility

- **Existing ADRs (ADR-001 through ADR-066) are NOT renamed.** They keep their sequential IDs.
- All parsers MUST accept both formats: `ADR-\d{3}` (legacy) and `ADR-\d{10}` (timestamp).
- The unified regex pattern is `ADR-(\d+)` — already used in both Rust and TypeScript parsers.
- Sorting: timestamp IDs naturally sort after all legacy sequential IDs (since `2603221500 > 066`), preserving chronological order.

### ID Generation

The `hex adr schema` command SHALL:
1. Generate the next ID as `YYMMDDHHMM` from the current local time
2. No longer scan for `max + 1`
3. No longer require SpacetimeDB reservation (timestamps are inherently unique)
4. Display both the timestamp ID and its human-readable date interpretation

### Cross-References

When referencing ADRs in prose or code comments, use the full numeric ID:
- Legacy: `ADR-012` (unchanged)
- New: `ADR-2603221500`

The `hex adr search` and `hex adr status` commands accept either format.

### Collision Window

Two ADRs created in the same minute would still collide. This is acceptable because:
- ADR creation is a deliberative act (minutes apart, not seconds)
- If a same-minute collision occurs, append a single alpha suffix: `ADR-2603221500a`

## Consequences

**Positive:**
- Eliminates race conditions in concurrent ADR creation
- Works offline — no SpacetimeDB reservation needed
- Embeds creation timestamp in the filename (self-documenting)
- No migration required for existing ADRs
- Cross-project ADR references are naturally unique

**Negative:**
- IDs are longer (10 digits vs 3 digits) — slightly less ergonomic in prose
- Two numbering schemes coexist permanently (legacy sequential + new timestamp)
- Minute-granularity leaves a theoretical (but impractical) collision window

**Mitigations:**
- The `hex adr schema` command shows both the ID and its human-readable date
- All tooling already uses `ADR-\d+` regex, so no parser breakage
- Document the format in TEMPLATE.md so human authors generate correct IDs

## Implementation

| Phase | Description | Status |
|-------|------------|--------|
| P1 | Update `find_next_adr_number()` in `hex-cli/src/commands/adr.rs` to generate YYMMDDHHMM | Done |
| P2 | Update `schema()` display and remove SpacetimeDB reservation dependency | Done |
| P3 | Update `extractId()` and sort logic in `src/adapters/secondary/adr-adapter.ts` | Done |
| P4 | Update workplan schema regex in `docs/workplan-schema.json` | Done |
| P5 | Update `TEMPLATE.md` placeholder and `/hex-adr-create` skill | Done |
| P6 | Update `hex-cli/src/commands/adr.rs` ID column width for 10-digit IDs | Done |
| P7 | Fix existing collision: resolved — second file already uses timestamp ID (`ADR-2603221522`) | Done |

## References

- ADR-012: ADR Lifecycle Tracking (extended by this decision)
- Existing collision: `ADR-059-canonical-project-identity.md` and `ADR-2603221522-embedded-asset-bundle.md`
- Prior art: [ULID](https://github.com/ulid/spec), [Snowflake IDs](https://en.wikipedia.org/wiki/Snowflake_ID) — similar timestamp-based uniqueness strategies

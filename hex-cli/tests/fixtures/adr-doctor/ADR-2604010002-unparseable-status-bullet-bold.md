# ADR-2604010002: Bullet-prefixed bold-colon-outside Status

- **Status**: Proposed
- **Date**: 2026-04-02
- **Drivers**: Reproduction of the buggy frontmatter format that slipped past `hex adr list` for >10 days. The bullet plus colon-outside-bold form does not parse with the strict status reader, so it must be flagged as `UnparseableStatus`.

## Context

This is the exact shape that triggered the session referenced in ADR-2026-04-27-0800. It is syntactically valid markdown but the doctor must reject it because downstream tooling (e.g. `hex adr status`) cannot recover the lifecycle state from it.

## Decision

Flag this and let Tier-A auto-fix rewrite it to canonical form in P2.

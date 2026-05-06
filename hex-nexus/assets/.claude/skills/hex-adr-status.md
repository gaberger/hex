---
name: hex-adr-status
description: Check ADR lifecycle -- find stale, abandoned, or conflicting decisions
trigger: /hex-adr-status
---

# ADR Status Report

## Steps

1. Read all ADR files from `docs/adrs/`

2. Parse status, date, and references from each

3. Check for issues:
   - **Stale**: Proposed ADRs older than 30 days without resolution
   - **Abandoned**: ADRs with no related code changes in the last 90 days
   - **Conflicting**: Multiple accepted ADRs that contradict each other
   - **Missing references**: ADRs that mention other ADRs that don't exist
   - **Numbering gaps**: Missing numbers in the sequence
   - **Duplicate numbers**: Same number used twice

4. Generate report:
   ```
   ADR Health Report
   -----------------
   Total: 45 ADRs
   Accepted: 38 | Proposed: 4 | Superseded: 2 | Deprecated: 1

   Issues Found:
   - ADR-032 has duplicate numbers (two different files)
   - ADR-041 has duplicate numbers (review agent + spacetimedb)
   - 3 proposed ADRs older than 30 days
   ```

5. Offer to fix issues (renumber duplicates, update statuses)

## CLI Shortcut

The `hex adr` subcommands can assist:
- `hex adr list` -- list all ADRs with status
- `hex adr status <id>` -- show detail for one ADR
- `hex adr abandoned` -- detect stale/abandoned ADRs

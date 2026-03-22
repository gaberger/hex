---
name: hex-adr-review
description: Review code changes against existing Architecture Decision Records
trigger: /hex-adr-review
---

# Review Code Against ADRs

## Steps

1. Get the current git diff: `git diff --cached` or `git diff HEAD`

2. Read all ADR files from `docs/adrs/` to understand architectural decisions

3. For each changed file, check:
   - Does it violate any boundary rules from ADRs?
   - Does it contradict any accepted decisions?
   - Should a new ADR be written for this change?

4. Report findings:
   - **Compliant**: changes align with existing ADRs
   - **Warning**: changes touch areas covered by ADRs but may not violate them
   - **Violation**: changes directly contradict an accepted ADR
   - **New ADR needed**: significant architectural change without an ADR

## Key ADRs to Check
- ADR-001: Hexagonal architecture boundaries
- ADR-014: Dependency injection (no mock.module)
- ADR-025: SpacetimeDB as state backend
- ADR-042: SpacetimeDB single source of truth

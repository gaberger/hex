# ADR-2604010016: ADR pointing at a non-existent dependency

**Status:** Accepted
**Date:** 2026-04-16
**Depends on:** ADR-2099-99-99-9999 (intentionally absent from the corpus)
**Drivers:** Positive case for `DanglingDependency`. The detector walks the corpus, sees the cited ID is not present, and flags.

## Context

Catches the most common drift mode: an ADR is renamed or deleted but referencing ADRs aren't updated.

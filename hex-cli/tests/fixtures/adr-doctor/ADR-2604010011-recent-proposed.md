# ADR-2604010011: Recently proposed ADR (not stale)

**Status:** Proposed
**Date:** 2026-04-25
**Drivers:** Negative control for `StaleProposed`. Date is two days before the test's frozen `now`, well inside the 30-day window.

## Context

Verifies the detector does not fire on freshly-proposed ADRs.

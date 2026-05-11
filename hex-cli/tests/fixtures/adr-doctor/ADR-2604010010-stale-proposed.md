# ADR-2026-04-01-0010: Stale proposed ADR

**Status:** Proposed
**Date:** 2025-01-01
**Drivers:** ADR-012 says proposed ADRs decay after 30 days without acceptance. This fixture is dated >1 year before the test's frozen `now`, so the detector must fire.

## Context

A real-world example: an ADR drafted, opened for review, and silently abandoned. The doctor catches it so the daemon can escalate.

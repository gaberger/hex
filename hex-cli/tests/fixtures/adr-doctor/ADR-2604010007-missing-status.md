# ADR-2026-04-01-0007: ADR with no Status field

**Date:** 2026-04-07
**Drivers:** Verify that a file with no Status line at all triggers `MissingRequiredField` and NOT `UnparseableStatus`. The two are mutually exclusive.

## Context

Stripped-down ADR used to exercise the missing-field branch. Status simply isn't present.

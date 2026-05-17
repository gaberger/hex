# ADR-2026-04-01-0008: ADR with no Date field

**Status:** Accepted
**Drivers:** Date is absent. Should produce exactly one `MissingRequiredField` finding (Date), nothing else.

## Context

Confirms the detector reports each missing field independently rather than collapsing them.

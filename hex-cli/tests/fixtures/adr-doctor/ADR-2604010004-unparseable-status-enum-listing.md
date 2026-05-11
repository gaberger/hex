# ADR-2026-04-01-0004: Inline status enum listing

**Status:** Proposed | Accepted | Deprecated
**Date:** 2026-04-04
**Drivers:** Some agents copy the status enum into the field instead of picking one. The classifier counts known keywords; a value containing more than one is rejected.

## Context

This is the second pattern seen in production: the agent literally copied the schema's enum line into the rendered ADR. The doctor must flag this because there is no single lifecycle state to act on.

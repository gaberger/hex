# ADR-2099-99-99-9999: Filename and H1 disagree

**Status:** Accepted
**Date:** 2026-04-06
**Drivers:** Catch the case where someone renames the file but forgets to update the H1, or vice versa. Either way the registry is in an inconsistent state.

## Context

The filename's ADR ID is `ADR-2026-04-01-0006`; the H1 above claims `ADR-2099-99-99-9999`. The detector compares both and flags any divergence.

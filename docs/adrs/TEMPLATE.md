# ADR-{YYMMDDHHMM}: {Title}

**Status:** Proposed
**Date:** {YYYY-MM-DD}
**Epoch:** {current design era — e.g. single-agent. Omit to derive from Date. See ADR-2606071243}
**Drivers:** {What triggered this decision — a bug, a requirement, a constraint}
**Supersedes:** {ADR-YYMMDDHHMM if replacing an earlier decision}
**Superseded-By:** {ADR-YYMMDDHHMM if a later decision replaces this one}

<!-- ID format: YYMMDDHHMM — use your local time. Example: 2603221500 = 2026-03-22 15:00 -->

<!-- LIFECYCLE (owned by the adr-steward agent; change Status only via the
     adr_status_set tool):
       Proposed → Accepted → Completed        (the success path)
       Proposed → Rejected | Abandoned        (declined / went stale >30d)
       Accepted|Completed → Superseded | Deprecated
     • Accepted   = the decision is approved.
     • Completed  = Accepted AND implementation CONFIRMED — its workplan
                    reconciles done (`hex plan reconcile`) and evidence passes.
                    Completed authorizes adapters exactly like Accepted.
     • Superseded MUST carry a `Superseded-By: ADR-XXX` backlink. -->

## Context

{Describe the problem or situation that requires a decision. Include:
- What is the current state?
- What forces are at play (technical, business, team)?
- What constraints exist?
- What alternatives were considered?}

## Decision

{State the decision clearly and concisely. This is the core of the ADR.
Use imperative language: "We will...", "The system shall...", "All X must..."

Include:
- The chosen approach
- Key design details
- Boundaries and scope}

## Consequences

**Positive:**
- {Benefit 1}
- {Benefit 2}

**Negative:**
- {Tradeoff 1}
- {Tradeoff 2}

**Mitigations:**
- {How negative consequences are addressed}

## Implementation

{Optional: specific implementation guidance}

| Phase | Description | Status | Verification |
|-------|------------|--------|--------------|
| P1 | {First step} | {Pending/Done} | code:{path}, test:{command} |
| P2 | {Second step} | {Pending/Done} | code:{path}, test:{command} |

<!--
Verification column format: `code:<path>, test:<command>` (per ADR-2026-03-31-2340)
- code:<path>   — file path that must exist when phase is Done (e.g. code:hex-cli/src/commands/foo.rs)
- test:<command> — shell command that must succeed (e.g. test:cargo test -p hex-cli foo)
Either field is optional. Leave the column blank for phases without automated verification.
-->


## References

- {Link to related ADRs, issues, documents}

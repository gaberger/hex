# ADR-2603312350: Internal Architecture Research Initiative

**Status:** Abandoned — Too broad; specific ADRs cover each concern
**Date:** 2026-03-31
**Drivers:** hex has 80+ ADRs but:
- Workflows are fragile (unexplained failures, incomplete error handling)
- Audit trails are incomplete (tool calls, cost aggregation, agent activity not fully tracked)
- Core hexagonal architecture principles not fully achieved (boundary violations still occur)

We lack systematic understanding of why these gaps exist and how to close them.

## Context

### Current State

| Area | Status | Gap |
|------|--------|-----|
| Workflow reliability | Partial | `hex dev` pipelines fail unexpectedly; error messages are unclear |
| Audit trail | Partial | Tool calls logged locally but not in SpacetimeDB (ADR-2603232230 pending) |
| Architecture enforcement | Partial | `hex analyze` detects violations but doesn't auto-fix; some ADRs unimplemented |

### Why we need research

1. **No systematic diagnosis** — We fix symptoms, not root causes
2. **Cascading failures** — One workflow failure triggers multiple downstream issues
3. **Incomplete instrumentation** — Can't observe what's happening in production
4. **ADRs drift** — Many "Proposed" ADRs from months ago still not implemented

## Decision

Form an internal **Architecture Research Team (ART)** to systematically investigate gaps and create remediation ADRs.

### Team Composition

- 1 Lead: Responsible for triage, prioritization, ADR creation
- 2 Researchers: Deep-dive into specific areas
- Rotating participation from feature developers

### Research Focus Areas

#### 1. Workflow Reliability (Priority: High)

Investigate:
- What causes `hex dev` pipeline failures?
- Where does error handling break down?
- Are there race conditions in HexFlo coordination?
- Does SpacetimeDB disconnection cause cascade failures?

Output: ADR(s) with specific fixes

#### 2. Audit Trail Gaps (Priority: High)

Investigate:
- What data should be tracked that isn't?
- Are current logs sufficient for debugging?
- Can we correlate tool calls to agent actions?
- Cost tracking completeness

Output: ADR(s) specifying complete audit trail schema

#### 3. Architecture Enforcement (Priority: Medium)

Investigate:
- Why do boundary violations still occur?
- Is `hex analyze` catching all issues?
- Should enforcement be automatic vs. advisory?
- Are ADRs being followed in code generation?

Output: ADR(s) for stronger enforcement

### Process

1. **Triage meeting** (1hr/week): Review known issues, prioritize
2. **Deep-dive** (2-4 days per area): Collect data, interview developers, analyze logs
3. **Gap report**: Document findings with evidence
4. **Remediation ADR**: Create ADRs for each fix
5. **Implementation tracking**: Add to existing workplan system

### Deliverables

| Week | Deliverable |
|------|--------------|
| 1 | Triage report: Top 10 workflow issues, top 5 audit gaps, top 5 arch gaps |
| 2 | Deep-dive: Workflow reliability report |
| 3 | Deep-dive: Audit trail gaps report |
| 4 | Deep-dive: Architecture enforcement report |
| 5 | Draft remediation ADRs for all findings |
| 6+ | Track implementation of remediation ADRs |

## Consequences

**Positive:**
- Systematic understanding of architectural debt
- Prioritized remediation based on evidence, not guesswork
- ADRs that actually solve root causes
- Knowledge transfer to broader team

**Negative:**
- 2-4 weeks of research time before visible progress
- May surface issues that are hard to fix

**Mitigations:**
- Continue minor fixes in parallel with research
- Focus on high-impact gaps first

## Implementation

| Phase | Description | Status |
|-------|------------|--------|
| P1 | Form ART (assign lead + 2 researchers) | Pending |
| P2 | Week 1 triage meeting and initial report | Pending |
| P3 | Weeks 2-4 deep-dives | Pending |
| P4 | Week 5 draft remediation ADRs | Pending |
| P5 | Week 6+ track implementation | Pending |

## References

- ADR-012: ADR Lifecycle Tracking
- ADR-2603232230: Tool Call Tracking in SpacetimeDB (pending)
- ADR-2603312340: Automated ADR Acceptance Verification
- ADR-2603241800: Swarm Lifecycle Management (pending)

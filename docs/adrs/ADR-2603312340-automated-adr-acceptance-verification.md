# ADR-2603312340: Automated ADR Acceptance Verification

**Status:** Proposed
**Date:** 2026-03-31
**Drivers:** ADRs document architectural decisions but have no automated verification that implementation matches the spec. Status is manually changed from "Proposed" to "Accepted" with no test or validation — unlike hex's enforcement of hexagonal architecture on target projects.

## Context

Current ADR lifecycle has a critical gap:
1. ADR is written with implementation phases (P1, P2, etc.)
2. Someone implements the phases
3. Someone manually changes status to "Accepted"
4. There's no verification that:
   - The code actually exists
   - Tests pass
   - The feature works as specified

This contrasts with hex's enforcement of hexagonal architecture — target projects get validated, but hex's own ADRs don't.

### Problems observed

- ADRs marked "Accepted" but implementation phases still show "Pending"
- No connection between ADR phases and actual code (worktrees, commits)
- "Abandoned detection" (ADR-012) only checks file age, not completion
- No way for an agent to verify "is this ADR done?"

## Decision

Add automated ADR acceptance verification that runs during the normal hex pipeline:

### 1. Implementation Phase Linking

Each implementation phase in an ADR references:
- Worktree branch (e.g., `worktree: feat/adr-verification`)
- Commit hash (when complete)
- Test file or command that validates the phase

Example:
```markdown
| P1 | Add `dev_tool_call` table to SpacetimeDB | Done | worktree: feat-tool-calls, commit: abc123 |
| P2 | Add REST endpoint `/api/hexflo/tool-calls` | Pending | |
```

### 2. Verification Commands

```bash
hex adr verify ADR-2603232230  # Verify single ADR
hex adr verify --all           # Verify all pending ADRs
hex adr verify --stale          # Verify ADRs with no updates in 30 days
```

Verification checks:
1. **Code exists** — referenced files/paths exist
2. **Tests pass** — referenced test commands succeed
3. **Status matches** — if all phases "Done", suggests status change to "Accepted"
4. **Worktree merged** — if referenced worktrees exist, warn if not merged

### 3. MCP Tool: `hex_adr_verify`

```json
{
  "adr_id": "ADR-2603232230",
  "verify_code": true,
  "verify_tests": true
}
```

Returns:
```json
{
  "adr_id": "ADR-2603232230",
  "phases": [
    { "phase": "P1", "status": "Done", "code_exists": true, "tests_pass": true },
    { "phase": "P2", "status": "Pending", "code_exists": false, "tests_pass": null }
  ],
  "ready_for_acceptance": false,
  "blocking_issues": ["P2: code not found at hex-nexus/src/routes/tool_calls.rs"]
}
```

### 4. Automatic Status Suggestions

When all phases verify as done:
- CLI/MCP returns a warning: "All phases complete — consider changing status to Accepted"
- Dashboard shows a "Promote to Accepted" button
- `hex adr abandoned` excludes ADRs that are verified complete but awaiting manual status change

### 5. Phase Status Schema

Enhance the implementation table format to include verification metadata:

```markdown
| Phase | Description | Status | Verification |
|-------|------------|--------|--------------|
| P1 | Add table | Done | code:hexflo/src/lib.rs:180, test:unit |
| P2 | Add endpoint | Pending | |
```

Verification column format: `code:<path>, test:<command>`

## Consequences

**Positive:**
- ADRs are self-verifying — reduces drift between spec and implementation
- Agents can query "is this ADR done?" programmatically
- Dashboard shows accurate completion status
- Complements ADR-012 (abandoned detection) with completion detection

**Negative:**
- Requires worktree/commit references in ADR — extra bookkeeping
- Test verification may be flaky if tests are unstable
- "Verification" is only as good as the metadata — garbage in, garbage out

**Mitigations:**
- Verification is opt-in per phase (can omit code/test references)
- Warn but don't block on verification failures
- Allow manual override: `hex adr verify --force-accept ADR-xxx`

## Implementation

| Phase | Description | Status |
|-------|------------|--------|
| P1 | Add verification column schema to TEMPLATE.md | Pending |
| P2 | Add `hex adr verify` CLI command | Pending |
| P3 | Add MCP tool `hex_adr_verify` | Pending |
| P4 | Add verification to hex-nexus REST API | Pending |
| P5 | Dashboard: verification status panel | Pending |
| P6 | Auto-suggest status change when all phases verify | Pending |

## References

- ADR-012: ADR Lifecycle Tracking
- ADR-2603232340: Validate Loop (related — automated testing)
- hex analyze: existing validation for hexagonal architecture

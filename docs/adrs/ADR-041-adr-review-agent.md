# ADR-041: ADR Review Agent — Architectural Consistency Guardian

**Status:** Accepted
**Accepted Date:** 2026-03-22
## Date: 2026-03-21

> **Implementation Evidence:** All 5 checks implemented in `hex-cli/src/commands/adr_review.rs`: scope conflict detection (keyword overlap), supersession chain validation, duplicate numbering, stale reference scanning (CLAUDE.md, skills, agents), metadata validation. CLI: `hex adr review [ADR-NNN] [--strict]`. LLM-powered semantic analysis deferred to future phase.
- **Informed by**: ADR-040 (scope creep observed), ADR-035 (v2 architecture shift), ADR-032 (duplicate numbering incident)
- **Authors**: Gary (architect), Claude (analysis)

## Context

### The Problem

hex has 41 ADRs spanning 6 months of rapid development. New ADRs are written to capture architectural decisions, but **no systematic process checks whether a new ADR contradicts, supersedes, or invalidates previous ones**. This creates three failure modes:

1. **Stale guidance**: CLAUDE.md references ADR patterns that a later ADR has abandoned. Code generation follows the stale pattern.
2. **Scope overlap**: Two ADRs claim authority over the same architectural boundary (e.g., ADR-024 and ADR-027 both define swarm coordination, with ADR-027 intended to supersede ADR-024 but the supersession wasn't enforced everywhere).
3. **Silent contradiction**: A new ADR assumes a pattern that conflicts with an earlier decision. Neither is marked as superseded. Agents pick whichever they encounter first.

### Real Examples

| Issue | ADRs Involved | Impact |
|-------|--------------|--------|
| Duplicate numbering | ADR-032 appears twice (deprecate-hex-hub AND sqlite-to-spacetimedb) | Confusion in `hex adr list` |
| Supersession gap | ADR-024 (hex-hub) → ADR-027 (HexFlo) → ADR-035 (v2 Rust-first) each redefine coordination, but ADR-024 is never marked superseded | Agent code still references hex-hub patterns |
| Scope creep | ADR-040 (transport layer) grew to include spawn protocol, inference routing, and CLI commands — arguably 3 ADRs | Workplan expanded from 13 to 22+ steps |

### Why This Matters for Code Generation

Every ADR feeds into the development pipeline through:
- **CLAUDE.md**: References specific ADRs as authoritative
- **Skills**: `/hex-generate` reads ADR patterns to generate conformant code
- **Agents**: `hex-coder` and `planner` use ADR context for architectural decisions
- **hex analyze**: Boundary rules derived from ADR-defined layer contracts

If ADRs contradict each other, the pipeline generates architecturally inconsistent code.

## Decision

### 1. ADR Review Agent

Create an autonomous agent (`adr-reviewer`) that runs after any ADR is created or updated. It performs five checks:

#### Check 1: Scope Conflict Detection

Compare the new ADR's domain against all existing ADRs to find overlapping authority:

```
Input:  ADR-040 "Remote Agent Transport"
Scan:   All ADRs mentioning "agent", "transport", "SSH", "WebSocket", "remote"
Find:   ADR-037 "Agent Lifecycle — Local Default + Remote Connect"
Report: OVERLAP — both ADRs define remote agent connection semantics.
        ADR-037 §2 (Remote Agent Connect) overlaps with ADR-040 §3 (WebSocket Protocol).
Action: ADR-040 should explicitly reference ADR-037 and clarify which aspects it supersedes.
```

#### Check 2: Architectural Drift Detection

Verify the new ADR's patterns are consistent with the established architecture:

```
Input:  New ADR proposes "routes import use cases directly"
Scan:   ADR-035 §Hexagonal Rules: "primary adapters import ports only"
Report: CONTRADICTION — new ADR violates hex boundary rule from ADR-035.
Action: Either update ADR-035 to allow this exception, or fix the new ADR.
```

#### Check 3: Supersession Chain Validation

Walk the supersession graph and flag gaps:

```
Input:  ADR-027 says "Informed by: ADR-024"
Check:  Is ADR-024 marked as Superseded by ADR-027?
Report: GAP — ADR-024 status is still "Accepted", should be "Superseded by ADR-027"
Action: Update ADR-024 status.
```

#### Check 4: Stale Reference Scan

Check all files that reference ADRs for outdated references:

```
Scan:   CLAUDE.md, skills/*.md, agents/*.yml, src/**/*.rs
Find:   CLAUDE.md line 42 references "ADR-024 hex-hub coordination"
Check:  ADR-024 is superseded by ADR-027
Report: STALE_REF — CLAUDE.md references superseded ADR-024
Action: Update CLAUDE.md to reference ADR-027 instead.
```

#### Check 5: Numbering and Metadata Validation

```
Check:  No duplicate ADR numbers
Check:  All ADRs have Status, Date, Authors fields
Check:  Status is one of: Proposed, Accepted, Superseded, Abandoned, Deferred
Check:  Superseded ADRs reference their successor
Report: META — ADR-032 has duplicate numbering (two files with ADR-032 prefix)
```

### 2. Trigger Points

| Trigger | Action |
|---------|--------|
| New ADR file created | Full review against all existing ADRs |
| ADR status changed | Supersession chain validation |
| `hex adr review` CLI command | On-demand full review |
| Pre-commit hook (optional) | Block commit if CRITICAL findings exist |
| Feature dev start (`/hex-feature-dev`) | Review referenced ADRs for staleness |

### 3. Output Format

```json
{
  "reviewed_adr": "ADR-040",
  "timestamp": "2026-03-21T14:00:00Z",
  "findings": [
    {
      "severity": "WARNING",
      "check": "scope_conflict",
      "adr_a": "ADR-040",
      "adr_b": "ADR-037",
      "description": "Both define remote agent connection semantics",
      "recommendation": "ADR-040 should reference ADR-037 §2 and clarify supersession"
    },
    {
      "severity": "CRITICAL",
      "check": "stale_reference",
      "file": "CLAUDE.md",
      "line": 42,
      "description": "References superseded ADR-024",
      "recommendation": "Update to reference ADR-027"
    }
  ],
  "verdict": "NEEDS_ACTION",
  "blocking": true
}
```

Severity levels:
- **CRITICAL**: Contradiction or stale reference in pipeline files (CLAUDE.md, skills, agents). **Blocks development.**
- **WARNING**: Scope overlap or missing supersession. Should be resolved before next feature.
- **INFO**: Style issues, missing metadata. Non-blocking.

### 4. Implementation Architecture

```
hex-nexus/src/
  usecases/
    adr_reviewer.rs          # Core review logic (use case)
  ports/
    adr_review.rs            # IAdrReviewPort trait
  adapters/
    adr_review_adapter.rs    # Filesystem + LLM implementation

hex-cli/src/commands/
  adr.rs                     # hex adr review command (extend existing)

agents/
  adr-reviewer.yml           # Agent definition for Claude Code integration
```

The reviewer use case depends on:
- `IFileSystemPort` — read ADR files and pipeline files
- `IInferencePort` — LLM-powered semantic comparison (scope overlap, contradiction detection)
- `IAdrReviewPort` — output report

### 5. LLM-Powered Semantic Analysis

Simple string matching won't catch architectural drift. The agent uses LLM inference for:

1. **Scope extraction**: Summarize each ADR's domain in 2-3 sentences
2. **Pairwise comparison**: "Does ADR-X contradict or overlap with ADR-Y?"
3. **Consistency check**: "Does this pattern conform to the architectural rules in ADR-035?"

This runs through the existing inference infrastructure (Ollama on bazzite for cost-effective local inference, Anthropic for high-stakes reviews).

### 6. Integration with Existing `hex adr` Commands

Extend the existing CLI:

```bash
hex adr review              # Review all ADRs, report findings
hex adr review ADR-040      # Review specific ADR against all others
hex adr review --fix        # Auto-fix metadata issues (numbering, status)
hex adr review --strict     # Exit non-zero if any WARNING+ findings (for CI)
hex adr abandoned           # Existing — detect stale ADRs (enhanced with reviewer data)
```

## Consequences

### Positive
- **Pipeline integrity**: Stale ADR references caught before they corrupt code generation
- **Governance at scale**: 41+ ADRs reviewed systematically, not by human memory
- **Supersession enforcement**: Can't silently abandon an ADR — the chain must be explicit
- **CI integration**: `hex adr review --strict` in pre-commit or PR checks

### Negative
- **LLM cost**: Semantic comparison requires inference calls (~1-2K tokens per ADR pair)
- **False positives**: LLM may flag legitimate overlaps as conflicts
- **Maintenance**: Reviewer rules must evolve as architectural patterns change

### Risks
- **Review fatigue**: Too many WARNING findings leads to ignoring the tool
- **LLM hallucination**: Semantic comparison may invent contradictions that don't exist
- **Circular dependency**: Reviewer references ADRs that it's reviewing

### Mitigations
- Cache pairwise comparison results — only re-run when either ADR changes
- Use high-confidence threshold (>0.8) before reporting conflicts
- Human confirmation required for CRITICAL findings before blocking pipeline
- Reviewer's own rules are in this ADR, not in the ADRs it reviews

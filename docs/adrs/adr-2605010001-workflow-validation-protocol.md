# ADR-2026-05-01-0001: Workflow Validation Protocol

**Status**: Accepted  
**Date**: 2026-05-01  
**Context**: Workplan execution system needs standardized validation methodology  
**Decision**: Establish formal protocol for testing autonomous workplan execution  

---

## Context

The hex workplan automation system orchestrates multi-phase, multi-task feature development through autonomous agent coordination. Prior to 2026-05-01, no formal validation protocol existed to verify:

1. End-to-end execution correctness
2. Architectural boundary compliance
3. Quality gate effectiveness
4. Self-healing capabilities
5. Performance characteristics

Without standardized validation, we cannot confidently scale autonomous development or measure system improvements.

---

## Decision

Establish a **Workflow Validation Protocol** with the following components:

### 1. Test Workplan Requirements

All validation workplans MUST include:

- **Multi-phase structure** (≥3 phases) testing sequential orchestration
- **Cross-layer implementation** (domain → port → adapter minimum)
- **Evidence requirements** for each task (grep patterns, compilation, tests)
- **Quality gates** (compilation checks, test execution, architecture analysis)
- **Documentation phase** (ADR or spec generation)

Example structure:
```json
{
  "id": "test-{feature}",
  "phases": [
    { "title": "Domain", "tasks": [...] },
    { "title": "Port", "tasks": [...] },
    { "title": "Adapter", "tasks": [...] },
    { "title": "Test", "tasks": [...] },
    { "title": "Docs", "tasks": [...] }
  ]
}
```

### 2. Execution Validation

Every workplan execution MUST be validated across:

#### Phase 1: Execution Metrics
- Total duration
- Tasks completed / total tasks
- Phases completed / total phases
- Commits generated
- Agent spawn count
- Human interventions (target: 0)

#### Phase 2: Code Quality
- Compilation: `cargo check --workspace`
- Tests: `cargo test -p {package} {feature}`
- Evidence: All task evidence requirements verified
- Coverage: Test suite includes edge cases

#### Phase 3: Architecture Compliance
- `hex analyze {crate}` boundary violation check
- Architecture grade (target: ≥B, 80/100)
- Layer dependency analysis
- Import graph validation

#### Phase 4: Self-Healing
- Violation detection post-execution
- Automatic correction capability
- Re-validation after fixes
- Backward compatibility preservation

### 3. Documentation Requirements

Each validation test MUST produce:

1. **Test Report** (`docs/analysis/workflow-test-{date}.md`)
   - Executive summary
   - Execution timeline
   - Deliverables generated
   - Validation evidence
   - Performance metrics
   - Lessons learned

2. **ADR** (if protocol changes, like this one)
   - Context
   - Decision
   - Consequences
   - Future implications

3. **Git Commits**
   - Atomic commits per task (target)
   - Descriptive messages with task IDs
   - Co-authored attribution

### 4. Success Criteria

A workplan execution is considered **VALIDATED** when:

- ✅ All tasks marked `done` with evidence verified
- ✅ All commits generated and pushed
- ✅ Compilation passes: `cargo check --workspace`
- ✅ Tests pass: 100% of generated tests green
- ✅ Architecture grade: ≥B (80/100)
- ✅ Boundary violations: ≤ baseline (no new violations)
- ✅ Documentation: Test report + ADR (if applicable)
- ✅ Human interventions: 0 (fully autonomous)

### 5. Validation Workflow

```bash
# 1. Execute workplan
hex plan execute docs/workplans/test-{feature}.json

# 2. Monitor (optional, notifications are automatic)
hex plan status test-{feature}

# 3. Validate compilation
cargo check --workspace

# 4. Validate tests
cargo test -p {package} {feature}

# 5. Validate architecture
hex analyze {crate}

# 6. Check commits
git log --oneline -10

# 7. Generate report
# (manual for now, will automate via `hex plan report --validate`)

# 8. Commit documentation
git add docs/analysis/ docs/adrs/
git commit -m "docs: Workflow validation test {date}"
```

---

## Consequences

### Positive

1. **Standardization**: Consistent methodology across all workplan tests
2. **Traceability**: Every validation produces auditable artifacts
3. **Confidence**: Objective pass/fail criteria reduce ambiguity
4. **Improvement**: Metrics enable tracking system evolution
5. **Onboarding**: New contributors understand validation expectations

### Negative

1. **Overhead**: Each test requires documentation (~30 min)
2. **Maintenance**: Protocol must evolve with system changes
3. **Tooling Gap**: Manual report generation (until automated)

### Mitigations

- **Overhead**: Template-based report generation reduces time
- **Maintenance**: ADRs track protocol evolution
- **Tooling**: Roadmap includes `hex plan report --validate` automation

---

## Implementation

### Immediate (Done ✅)
- [x] First validation test executed (test-domain-migration)
- [x] Test report template created
- [x] This ADR documenting protocol

### Short Term
- [ ] Create validation workplan template generator
- [ ] Add `hex plan report --validate` command
- [ ] CI integration: `hex ci --workplan test-*`
- [ ] Dashboard: validation test history view

### Long Term
- [ ] Automated report generation from execution logs
- [ ] Regression testing: compare against baseline
- [ ] Performance benchmarks: track throughput over time
- [ ] Self-improving validation: learn from failures

---

## Examples

### Baseline Test (2026-05-01)

**Workplan**: `test-domain-migration`  
**Result**: ✅ PASSED  
**Metrics**:
- Duration: 4m 24s
- Tasks: 6/6 (100%)
- Commits: 6
- Tests: 6/6 passed
- Architecture: B (80/100)
- Interventions: 0

**Report**: `docs/analysis/workflow-test-2026-05-01.md`

### Future Tests

Planned validation workplans:
- `test-primary-adapter` — CLI/HTTP input adapter development
- `test-use-case` — Multi-adapter composition
- `test-integration` — End-to-end feature with real I/O
- `test-refactor` — Safe cross-layer refactoring
- `test-parallel` — Concurrent task execution

---

## Alternatives Considered

### 1. Manual Validation (Rejected)
**Reason**: Not scalable, subjective, error-prone

### 2. Unit Tests Only (Rejected)
**Reason**: Doesn't validate orchestration, agent coordination, or self-healing

### 3. Continuous Fuzzing (Deferred)
**Reason**: Valuable but requires workplan generation capability (roadmap item)

---

## References

- Test execution log: `hex plan report c86b2e94-08a5-4fcc-ae9f-952c34a99673`
- Test report: `docs/analysis/workflow-test-2026-05-01.md`
- Workplan: `docs/workplans/test-domain-migration.json`
- Commits: `91a39a55..87e7a59d` (6 commits)

---

## Appendix: Validation Checklist

Use this checklist for every workplan validation test:

```markdown
## Pre-Execution
- [ ] Workplan includes ≥3 phases
- [ ] Evidence requirements defined for all tasks
- [ ] Quality gates specified (compilation, tests, analysis)
- [ ] hex-nexus running
- [ ] SpacetimeDB accessible
- [ ] Clean git state

## Execution
- [ ] Workplan executed: `hex plan execute {path}`
- [ ] Completion notification received
- [ ] Status checked: `hex plan status {id}`

## Validation
- [ ] Compilation: `cargo check --workspace`
- [ ] Tests: `cargo test -p {package}`
- [ ] Architecture: `hex analyze {crate}`
- [ ] Commits: `git log --oneline` (review)
- [ ] Evidence: All task requirements verified

## Documentation
- [ ] Test report created (`docs/analysis/workflow-test-{date}.md`)
- [ ] ADR created (if protocol changes)
- [ ] Commits reviewed for quality
- [ ] Documentation committed

## Success Criteria
- [ ] All tasks complete (100%)
- [ ] All tests passing (100%)
- [ ] Architecture ≥B (80/100)
- [ ] No new boundary violations
- [ ] Zero human interventions
- [ ] Report published
```

---

**Approved By**: System Validation (2026-05-01)  
**Next Review**: After 10 validation tests or 3 months

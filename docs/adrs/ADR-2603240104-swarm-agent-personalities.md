# ADR-2603240104: Swarm Agent Personalities — Specialized Roles with Context-Aware Prompting

**Status:** Proposed
**Date:** 2026-03-24
**Drivers:** hex dev generates code but operates as a single pipeline — one model, one prompt, one pass. Real development requires specialized roles: a coder who writes, a reviewer who critiques, a tester who validates, a documenter who explains. Each role needs different context, different prompts, and different success criteria. The v2 swarm infrastructure (ADR-2603232340) provides the coordination layer — this ADR defines the agents that run on it.

## Context

### Current state (v1)

```
hex dev → single agent → code → quality gate (compile/test/analyze)
```

One prompt template per phase. No code review. No UX review. No documentation beyond README scaffold. The quality gate checks mechanical compliance (compiles? tests pass? boundaries clean?) but not **quality** (is the code well-structured? are edge cases handled? is the API ergonomic?).

### Target state (v2)

```
Supervisor
  ├── hex-coder    → writes code per workplan step
  ├── hex-reviewer → reviews code for quality + patterns
  ├── hex-tester   → writes tests + validates behavior
  ├── hex-documenter → API docs, README, inline comments
  ├── hex-ux       → reviews frontend UX/accessibility
  ├── hex-analyzer → runs hex analyze, enforces boundaries
  └── hex-fixer    → fixes issues found by reviewer/tester/analyzer
```

Each agent is a **HexFlo task** with a specialized system prompt, context window, and success criteria.

## Decision

### 1. Agent Personality Definitions

Each agent personality is defined by:
- **Role**: What it does
- **System prompt**: Loaded from `hex-cli/assets/prompts/agent-<name>.md`
- **Context assembly**: What data it needs in its prompt
- **Success criteria**: How the supervisor knows the task is done
- **Model preference**: Which model type works best for this role

#### hex-coder

```yaml
role: Code Generator
task_type: CodeGeneration
model_preference: meta-llama/llama-4-maverick  # fast, cheap
context:
  - workplan_step: The specific step description + tier + adapter
  - port_interfaces: Port trait definitions relevant to this step
  - existing_code: Previously generated files in the same tier
  - architecture_rules: Hex boundary rules for this tier
  - language: TypeScript/Rust with project conventions
success_criteria:
  - File written to correct path (tier-appropriate)
  - No imports from forbidden layers
  - Exports match port interface signatures
```

#### hex-reviewer

```yaml
role: Code Reviewer
task_type: Reasoning
model_preference: deepseek/deepseek-r1  # reasoning for quality judgment
context:
  - source_file: The code to review
  - port_interface: The port this code implements
  - architecture_rules: Hex boundary rules
  - workplan_step: What this code was supposed to do
  - review_checklist:
    - Error handling (no swallowed errors, no bare unwrap/catch)
    - Edge cases (empty inputs, null, concurrent access)
    - Naming (clear, consistent, domain-aligned)
    - SOLID principles (single responsibility, dependency inversion)
    - Hex compliance (no cross-adapter imports, domain purity)
success_criteria:
  - Review report with PASS/FAIL + specific issues
  - Each issue has: severity (critical/warning/info), file, line, description, fix suggestion
  - PASS if zero critical issues
output_format:
  verdict: PASS | NEEDS_FIXES
  issues:
    - severity: critical
      file: src/adapters/secondary/db.ts
      line: 42
      description: "Swallows database connection error"
      fix: "Propagate error to caller or log with context"
```

#### hex-tester

```yaml
role: Test Writer & Runner
task_type: CodeGeneration
model_preference: meta-llama/llama-4-maverick
context:
  - source_file: The code to test
  - port_interface: The port contract (defines expected behavior)
  - test_patterns: Project's existing test patterns (London school, Deps pattern)
  - workplan_step: What behavior to validate
  - architecture_rules: No mock.module(), use dependency injection
success_criteria:
  - Test file written covering happy path + error cases
  - Tests execute successfully (bun test / cargo test)
  - Coverage of port interface methods
output_format: Complete test file
```

#### hex-documenter

```yaml
role: Documentation Generator
task_type: StructuredOutput
model_preference: meta-llama/llama-4-maverick
context:
  - adr_content: The architecture decision (why this app exists)
  - source_files: All generated code files (for API surface extraction)
  - workplan: Full workplan (for architecture overview)
  - port_interfaces: Public API surface
  - language: For install/run commands
success_criteria:
  - README.md with: overview, architecture diagram (text), quick start, API reference, development guide
  - Inline JSDoc/rustdoc on all public functions
  - CHANGELOG entry for the feature
output_format: Multiple files (README.md, docs/*.md, inline comments)
```

#### hex-ux

```yaml
role: UX/Accessibility Reviewer
task_type: Reasoning
model_preference: deepseek/deepseek-r1
context:
  - source_files: Frontend/CLI adapter code
  - user_description: Original feature request (the "ask")
  - ux_checklist:
    - CLI: clear help text, consistent flags, error messages with suggestions
    - API: consistent response shapes, proper HTTP status codes, pagination
    - Frontend: keyboard navigation, color contrast, responsive layout
success_criteria:
  - UX report with issues and recommendations
  - PASS if zero critical UX issues
applies_when: workplan has primary adapters (tier 2) with UI/CLI/API
```

#### hex-analyzer

```yaml
role: Architecture Enforcer
task_type: General
model_preference: null  # no inference — runs hex analyze
context:
  - output_dir: Directory to analyze
  - language: For parser selection
success_criteria:
  - Score >= 90 (Grade A)
  - Zero boundary violations
  - Zero circular dependencies
output_format: AnalyzeResult JSON
```

#### hex-fixer

```yaml
role: Issue Resolver
task_type: CodeEdit
model_preference: deepseek/deepseek-r1  # reasoning for understanding issues
context:
  - issue: The specific issue (from reviewer, tester, or analyzer)
  - source_file: The file to fix
  - port_interface: Expected behavior
  - architecture_rules: Hex boundary rules
  - fix_template: fix-compile.md, fix-tests.md, or fix-violations.md
success_criteria:
  - Fixed file that resolves the specific issue
  - No new issues introduced
output_format: Complete corrected file
```

### 2. Supervisor Orchestration

The supervisor drives the agent pipeline per tier:

```
For each tier in workplan:
  1. ASSIGN hex-coder tasks (parallel within tier)
     → Wait for all code steps to complete

  2. ASSIGN hex-reviewer (parallel per file)
     → If NEEDS_FIXES: assign hex-fixer, then re-review (max 2 rounds)

  3. ASSIGN hex-tester (parallel per file)
     → If tests fail: assign hex-fixer, then re-test (max 2 rounds)

  4. ASSIGN hex-analyzer (one task for whole tier)
     → If violations: assign hex-fixer per violation, re-analyze

  5. If tier has UI/CLI/API: ASSIGN hex-ux
     → If critical issues: assign hex-fixer, re-review

  6. If final tier: ASSIGN hex-documenter
     → Generate README, API docs, inline comments

  7. Quality gate: all agents passed? → advance to next tier
```

### 3. Context Assembly per Agent

Each agent's prompt is assembled from **specific sources**, not generic context. The supervisor knows what each agent needs:

```rust
pub struct AgentContext {
    /// System prompt template name (from assets/prompts/)
    pub prompt_template: String,
    /// Specific files to include in context
    pub source_files: Vec<String>,
    /// Port interfaces relevant to this task
    pub port_interfaces: Vec<String>,
    /// Architecture rules for the target tier
    pub boundary_rules: String,
    /// The workplan step being worked on
    pub workplan_step: Option<WorkplanStep>,
    /// Previous agent outputs (e.g., reviewer issues for fixer)
    pub upstream_output: Option<String>,
    /// Task-specific metadata
    pub metadata: HashMap<String, String>,
}
```

The supervisor builds the context before creating each HexFlo task:

```rust
impl Supervisor {
    fn build_coder_context(&self, step: &WorkplanStep) -> AgentContext {
        AgentContext {
            prompt_template: "agent-coder.md".into(),
            source_files: self.files_for_tier(step.tier),
            port_interfaces: self.ports_for_step(step),
            boundary_rules: self.rules_for_tier(step.tier),
            workplan_step: Some(step.clone()),
            upstream_output: None,
            metadata: hashmap! { "language" => self.language.clone() },
        }
    }

    fn build_reviewer_context(&self, file: &str, step: &WorkplanStep) -> AgentContext {
        AgentContext {
            prompt_template: "agent-reviewer.md".into(),
            source_files: vec![file.into()],
            port_interfaces: self.ports_for_step(step),
            boundary_rules: self.rules_for_tier(step.tier),
            workplan_step: Some(step.clone()),
            upstream_output: None,
            metadata: hashmap! { "review_checklist" => REVIEW_CHECKLIST.into() },
        }
    }

    fn build_fixer_context(&self, file: &str, issue: &str) -> AgentContext {
        AgentContext {
            prompt_template: "agent-fixer.md".into(),
            source_files: vec![file.into()],
            port_interfaces: vec![],
            boundary_rules: self.rules_for_tier(0), // all rules
            workplan_step: None,
            upstream_output: Some(issue.into()),
            metadata: hashmap! {},
        }
    }
}
```

### 4. Prompt Templates

New prompt templates in `hex-cli/assets/prompts/`:

| Template | Agent | Key Context |
|----------|-------|-------------|
| `agent-coder.md` | hex-coder | Step description, ports, existing code, tier rules |
| `agent-reviewer.md` | hex-reviewer | Source file, port contract, review checklist |
| `agent-tester.md` | hex-tester | Source file, port contract, test patterns |
| `agent-documenter.md` | hex-documenter | ADR, all files, workplan, port surface |
| `agent-ux.md` | hex-ux | Adapter code, UX checklist, user description |
| `agent-fixer.md` | hex-fixer | Issue description, source file, fix template |

### 5. HexFlo Task Types

Extend the swarm task with agent role:

```rust
// In task creation
{
    "title": "P0.1: Define domain entities",
    "agent_role": "hex-coder",        // NEW — which personality
    "context_hash": "abc123",          // NEW — context assembly fingerprint
    "upstream_tasks": ["review-P0.1"], // NEW — tasks this depends on
}
```

### 6. SpacetimeDB: Agent Performance per Role

Track success rates per agent role (extends ADR-2603240045):

```sql
SELECT agent_role,
       AVG(analyze_score) as avg_score,
       SUM(success) * 100.0 / COUNT(*) as success_rate,
       AVG(fix_iterations) as avg_fixes
FROM model_performance
GROUP BY agent_role;
```

This reveals which roles need better prompts or different models.

## Implementation

| Phase | Description | Status |
|-------|------------|--------|
| P1 | Agent prompt templates (6 new templates) | Pending |
| P2 | AgentContext struct + supervisor context builders | Pending |
| P3 | Supervisor orchestration (per-tier agent pipeline) | Pending |
| P4 | hex-reviewer agent with structured review output | Pending |
| P5 | hex-tester agent with test generation + execution | Pending |
| P6 | hex-documenter agent with README + API docs | Pending |
| P7 | hex-ux agent with UX review checklist | Pending |
| P8 | hex-fixer agent with issue→fix routing | Pending |
| P9 | Wire supervisor into hex dev headless + TUI modes | Pending |
| P10 | SpacetimeDB: track per-role performance | Pending |

## Consequences

### Positive
- **Quality beyond compilation** — code is reviewed, tested, documented, and UX-checked
- **Specialized context** — each agent gets exactly the data it needs, not a generic blob
- **Parallel execution** — reviewer + tester can run simultaneously per file
- **Learning per role** — RL discovers which models are best for reviewing vs coding vs testing
- **Audit trail** — every review, test, and fix is a tracked HexFlo task

### Negative
- **Higher cost** — more inference calls per app (review + test + docs)
- **Longer runtime** — multiple agent passes add time
- **Prompt engineering** — 6 new templates to tune

### Mitigations
- Review/test/doc agents use cheap models (llama-4-maverick) except reviewer (needs reasoning)
- Agents run in parallel where possible (reviewer + tester at same time)
- `--quick` mode skips review + docs, only runs coder + analyzer
- Prompt templates are editable — community can improve them

## References

- ADR-2603232340: Validate Loop Until Grade A
- ADR-2603232005: Self-Sufficient hex-agent with TUI
- ADR-2603240045: Free Model Performance Tracking
- ADR-027: HexFlo Native Coordination
- ADR-031: RL-Driven Model Selection

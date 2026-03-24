# ADR-2603240130: Declarative Swarm Agent Behavior from YAML Definitions

**Status:** Proposed
**Date:** 2026-03-24
**Drivers:** Agent behaviors are hardcoded in `supervisor.rs` — model selection, context assembly, feedback loops, quality thresholds. But rich YAML agent definitions already exist in `hex-cli/assets/agents/hex/hex/*.yml` with model tiers, context loading strategies, workflow phases, feedback loops, and quality gates. The supervisor should read these YAMLs to configure agent behavior declaratively, not imperatively.

## Context

### What exists in YAML (14 agent definitions)

Each agent YAML defines:

```yaml
name: hex-coder
model:
  tier: 2
  preferred: sonnet
  fallback: haiku
  upgrade_to: opus
  upgrade_condition: "feedback loop exceeds 3 iterations"

context:
  load_strategy:
    - level: L1, scope: "src/core/ports/**"   # lightweight overview
    - level: L2, scope: "src/core/ports/index.ts"  # full signatures
    - level: L3, scope: "{{current_edit_file}}"  # full source on demand
  token_budget:
    max: 100000
    allocation: { port_interfaces: 5000, domain_entities: 3000, ... }

constraints:
  - "Never import from other adapters"
  - "Every public method must have unit tests"

workflow:
  phases:
    - id: pre_validate (boundary check, blocking gate)
    - id: red (TDD — write failing tests)
    - id: green (minimal implementation)
    - id: refactor (clean up)
    - id: test_coverage_gate (verify test categories)
  feedback_loop:
    max_iterations: 5
    gates: [compile, lint, test]
    on_max_iterations: escalate

quality_thresholds:
  test_coverage: 80
  max_lint_warnings: 5
  max_file_lines: 500
```

### What the supervisor does today (hardcoded)

```rust
// supervisor.rs — all behavior is imperative
match role {
    "hex-coder" => { CodePhase::from_env().execute_step(...).await; }
    "hex-reviewer" => { ReviewerAgent::from_env().execute(&context).await; }
    // ... hardcoded dispatch
}
```

- Model selection: hardcoded `TaskType::CodeGeneration → llama-4-maverick`
- Context: hardcoded `files_for_tier(tier)` + `port_files()`
- Feedback loop: hardcoded 5 iterations, compile → test → analyze
- Quality: hardcoded Grade A threshold

### The gap

The YAML definitions have **richer behavior** than the supervisor implements:
- L1/L2/L3 context loading (supervisor just reads all files)
- Token budget allocation (supervisor truncates at 4KB per file)
- TDD workflow phases (supervisor doesn't enforce red → green → refactor)
- Lint gate (supervisor only checks compile + test + analyze)
- Upgrade conditions (supervisor doesn't escalate to better models)
- Commit templates (supervisor doesn't commit per-agent)

## Decision

### 1. Parse YAML Agent Definitions at Startup

```rust
pub struct AgentDefinition {
    pub name: String,
    pub agent_type: String,
    pub model: ModelConfig,
    pub context: ContextConfig,
    pub constraints: Vec<String>,
    pub workflow: WorkflowConfig,
    pub feedback_loop: FeedbackLoopConfig,
    pub quality_thresholds: QualityThresholds,
    pub inputs: HashMap<String, InputSpec>,
    pub outputs: HashMap<String, OutputSpec>,
}
```

Load from `hex-cli/assets/agents/hex/hex/*.yml` via `rust-embed` (already baked into binary).

### 2. Supervisor Reads Agent Definitions

```rust
impl Supervisor {
    pub fn new(output_dir: &str, language: &str) -> Self {
        let agents = AgentDefinition::load_all();  // from embedded YAMLs
        // ...
    }

    fn get_agent_def(&self, role: &str) -> &AgentDefinition {
        &self.agents[role]
    }
}
```

### 3. Model Selection from YAML

Instead of hardcoded `TaskType → model`:

```rust
fn select_model_for_agent(&self, agent: &AgentDefinition, iteration: u32) -> String {
    if iteration > 3 && agent.model.upgrade_condition_met() {
        agent.model.upgrade_to  // escalate to better model
    } else {
        agent.model.preferred  // use preferred model
    }
}
```

Map YAML model names (`sonnet`, `haiku`, `opus`) to OpenRouter model IDs.

### 4. Context Assembly from YAML

Instead of hardcoded `files_for_tier()`:

```rust
fn build_context_from_yaml(&self, agent: &AgentDefinition, step: &WorkplanStep) -> AgentContext {
    let mut context = AgentContext::default();
    let mut token_count = 0;

    for strategy in &agent.context.load_strategy {
        let budget = agent.context.token_budget.allocation.get(&strategy.purpose);
        let files = glob(strategy.scope, step);  // resolve {{target_adapter}} etc.

        for file in files {
            let content = match strategy.level {
                "L1" => summarize_ast(file),       // lightweight overview
                "L2" => read_signatures(file),      // full signatures
                "L3" => read_full(file),            // complete source
            };
            if token_count + estimate_tokens(&content) > budget {
                break;  // respect token budget
            }
            context.source_files.push((file, content));
            token_count += estimate_tokens(&content);
        }
    }
    context
}
```

### 5. Workflow Phases from YAML

Instead of the supervisor choosing what to do:

```rust
for phase in &agent.workflow.phases {
    match phase.id.as_str() {
        "pre_validate" => {
            let result = run_boundary_check(step, output_dir);
            if phase.gate.blocking && !result.pass {
                // Gate failed — run gate.on_fail instructions
                continue;  // retry after fix
            }
        }
        "red" => { /* TDD: generate failing tests */ }
        "green" => { /* TDD: minimal implementation */ }
        "refactor" => { /* Clean up */ }
        "test_coverage_gate" => { /* Verify test categories */ }
    }
}
```

### 6. Feedback Loop from YAML

```rust
for iteration in 1..=agent.feedback_loop.max_iterations {
    for gate in &agent.feedback_loop.gates {
        let command = gate.command.get(language);
        let result = run_command(command, output_dir, gate.timeout_ms);
        if !result.pass {
            // Use gate.on_fail as instructions for the fixer
            fix_agent.execute(FixTaskInput {
                error_context: result.output,
                fix_instructions: gate.on_fail.clone(),
                ...
            });
            break;  // restart gates from top
        }
    }
    if all_gates_pass { break; }
}

if !all_gates_pass {
    // agent.feedback_loop.on_max_iterations.action == "escalate"
    escalate_to_coordinator();
}
```

### 7. Quality Thresholds from YAML

```rust
fn check_quality(&self, agent: &AgentDefinition, output_dir: &str) -> bool {
    let coverage = measure_coverage(output_dir);
    let lint = run_lint(output_dir);
    let metrics = measure_code_metrics(output_dir);

    coverage >= agent.quality_thresholds.test_coverage
        && lint.warnings <= agent.quality_thresholds.max_lint_warnings
        && metrics.max_file_lines <= agent.quality_thresholds.max_file_lines
        && metrics.max_cyclomatic_complexity <= agent.quality_thresholds.max_cyclomatic_complexity
}
```

### 8. Swarm Composition as YAML

Define which agents participate in a swarm:

```yaml
# hex-cli/assets/swarms/dev-pipeline.yml
name: dev-pipeline
topology: hex-pipeline
agents:
  - role: hex-coder
    per: workplan_step      # one instance per step
  - role: hex-reviewer
    per: source_file        # one per generated file
    parallel_with: hex-tester
  - role: hex-tester
    per: source_file
  - role: hex-analyzer
    per: tier               # one per tier
  - role: hex-ux
    per: tier
    when: has_ui_adapters
  - role: hex-documenter
    per: swarm              # once at the end
    when: final_tier
objectives:
  - CodeGenerated
  - CodeCompiles
  - TestsPass
  - ReviewPasses
  - ArchitectureGradeA
  - UxReviewPasses
  - DocsGenerated
max_iterations_per_tier: 5
```

This is the single source of truth for swarm behavior — not code.

## Implementation

| Phase | Description | Status |
|-------|------------|--------|
| P1 | Parse YAML agent definitions into `AgentDefinition` structs (serde_yaml) | Pending |
| P2 | Supervisor reads agent defs at startup, replaces hardcoded dispatch | Pending |
| P3 | Model selection from YAML (preferred/fallback/upgrade) mapped to OpenRouter IDs | Pending |
| P4 | Context assembly from YAML load_strategy with L1/L2/L3 levels | Pending |
| P5 | Workflow phases from YAML (pre_validate → red → green → refactor) | Pending |
| P6 | Feedback loop from YAML (compile/lint/test gates with on_fail instructions) | Pending |
| P7 | Quality thresholds from YAML | Pending |
| P8 | Swarm composition YAML (dev-pipeline.yml) | Pending |
| P9 | Token budget enforcement from YAML allocation | Pending |

## Consequences

### Positive
- **Behavior is declarative** — change YAML, change agent behavior (no code changes)
- **Rich behavior already defined** — 14 YAML files with TDD phases, feedback loops, quality gates
- **Community configurable** — users can customize agent behavior without forking
- **Self-documenting** — YAML is the specification AND the implementation
- **Swarm composition as config** — define which agents run, when, and how

### Negative
- **YAML parsing adds complexity** — serde_yaml dependency, schema validation
- **Two sources of truth during migration** — hardcoded + YAML until fully wired
- **YAML schema evolution** — changes to schema need backward compatibility

### Mitigations
- serde_yaml is battle-tested and already used in the ecosystem
- Migration can be gradual — one agent at a time
- YAML schema versioned via `version:` field in each definition

## References

- ADR-2603240104: Swarm Agent Personalities
- ADR-2603232340: Validate Loop Until Grade A
- ADR-2603232005: Self-Sufficient hex-agent with TUI
- Agent YAML definitions: `hex-cli/assets/agents/hex/hex/*.yml`

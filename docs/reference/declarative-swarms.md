# Declarative Swarm & Agent Behaviour

Source ADR: 2603240130. Skill: `/hex-swarm`.

Agent and swarm behaviour is defined declaratively in YAML, not hardcoded in Rust. The supervisor reads these YAMLs at startup and drives everything from them.

## Agent YAMLs — `hex-cli/assets/agents/hex/hex/`

14 agent YAMLs define:

- Model selection: tier / preferred / fallback / upgrade
- Context loading: L1 AST summary, L2 signatures, L3 full source
- Workflow phases: TDD (red → green → refactor)
- Feedback loop gates: compile / lint / test with `on_fail` instructions
- Quality thresholds
- Input / output schemas

Schema varies by role:

- **Coders** (`hex-coder.yml`): `workflow.phases[]` with blocking gates + `feedback_loop` with compile/lint/test
- **Planners** (`planner.yml`): `workflow.steps[]` + `escalation` conditions
- **Reviewers / Validators**: simpler workflows, stricter thresholds

## Swarm YAMLs — `hex-cli/assets/swarms/`

Define which agents participate, their cardinality, parallelism, and objectives:

```yaml
# hex-cli/assets/swarms/dev-pipeline.yml
name: dev-pipeline
topology: hex-pipeline
agents:
  - role: hex-coder
    cardinality: per_workplan_step
    inference:
      task_type: code_generation
      model: preferred                 # from agent YAML
      upgrade: { after_iterations: 3, to: opus }
  - role: hex-reviewer
    cardinality: per_source_file
    parallel_with: hex-tester
objectives:
  - id: CodeCompiles
    evaluate: "cargo check / tsc --noEmit"
    required: true
  - id: TestsPass
    evaluate: "cargo test / bun test"
    required: true
iteration:
  max_per_tier: 5
  on_max_iterations: escalate
```

Available swarm behaviours: `dev-pipeline`, `quick-fix`, `code-review`, `refactor`, `test-suite`, `documentation`, `migration`.

## Embedding

All templates (agents, swarms, hooks, skills, helpers, MCP config) live in `hex-cli/assets/` and are baked into both hex-cli and hex-nexus via `rust-embed` at compile. hex-nexus extracts templates into target projects during `hex init`.

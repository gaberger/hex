---
name: hex-feature-dev
description: Start feature development with hex decomposition and worktree isolation. Use when the user asks to "develop a feature", "new feature", "implement feature", "feature dev", "start feature", or "add feature".
---

# Hex Feature Dev — Develop a Feature Across Hexagonal Boundaries

CRITICAL: Do NOT enter plan mode (EnterPlanMode). Proceed directly with execution.

## Invocation

This skill can be invoked three ways:

1. **Manually**: user runs `/hex-feature-dev` (explicit)
2. **Draft pickup**: user has a pending draft from `hex plan drafts list` —
   read the draft's `prompt` field and start from Phase 1 using that prompt
3. **Auto-invocation (ADR-2604110227)**: when `hex hook route` classifies a
   user prompt as **T3 Workplan** and there is no active workplan, it
   auto-creates a draft stub at `docs/workplans/drafts/draft-*.json` and
   surfaces it in hook output via a one-line `[HEX]` banner. When you see
   that banner, pick up the draft (`hex plan drafts list`), read the prompt
   field, and start this skill's flow from Phase 1. This is the normal
   "it just happened" path — users don't have to remember to type the skill
   name. Opt-outs: `HEX_AUTO_PLAN=0`, `.hex/project.json` →
   `workplan.auto_invoke.enabled: false`, or `hex skip plan` in the prompt.

## How Hex Treats Features

In hexagonal architecture, a "feature" is NOT a single vertical slice. It decomposes inside-out across layers:

```
Domain types → Port contracts → Use cases → Adapters (parallel) → Composition root → Integration tests
```

Each adapter gets its own git worktree. Port/domain changes merge first, then adapters fan out in parallel, then integration merges last. This is enforced by the planner agent's dependency ordering.

## Phase 0: Initialize HexFlo Swarm

CRITICAL: Before any work begins, initialize the HexFlo swarm for tracking.

```tool
mcp__hex__hex_hexflo_swarm_init({
  topology: "hierarchical",
  maxAgents: 8,
  strategy: "specialized"
})
```

Then reconcile any prior state:

```tool
mcp__hex__hex_hexflo_task_list()
```

Cross-reference task list against `git log --oneline -10`. If commits exist for tasks, mark them complete:

```tool
mcp__hex__hex_hexflo_task_complete({
  task_id: "...",
  result: "feat(adapter): summary — commit abc1234"
})
```

## Phase 1: Feature Discovery

Use AskUserQuestion to understand the feature scope.

```tool
AskUserQuestion({
  questions: [
    {
      question: "Describe the feature you want to build",
      header: "Feature Description",
      multiSelect: false,
      options: [
        { label: "Free-form description", description: "I'll describe what the feature should do" }
      ]
    },
    {
      question: "Which layers does this feature touch?",
      header: "Affected Layers",
      multiSelect: true,
      options: [
        { label: "Domain (new types/entities)", description: "New value objects, entities, or domain events" },
        { label: "Ports (new/modified interfaces)", description: "New port contracts or changes to existing ones" },
        { label: "Use cases (new orchestration)", description: "New application logic composing ports" },
        { label: "Primary adapters (CLI, HTTP, MCP)", description: "New ways to drive the application" },
        { label: "Secondary adapters (DB, API, FS)", description: "New infrastructure integrations" },
        { label: "Not sure — let the planner decide", description: "The planner agent will analyze and decompose" }
      ]
    },
    {
      question: "How should this feature be developed?",
      header: "Development Mode",
      multiSelect: false,
      options: [
        { label: "Swarm (recommended)", description: "Multi-agent parallel development with worktree isolation. Best for features spanning 2+ adapters." },
        { label: "Interactive", description: "Step-by-step with human review at each phase. Best for learning or critical features." },
        { label: "Single-agent", description: "One agent handles everything sequentially. Best for small, single-adapter features." }
      ]
    }
  ]
})
```

After discovery, store the feature context in HexFlo memory:

```tool
mcp__hex__hex_hexflo_memory_store({
  key: "feature/{{feature-name}}/context",
  value: {
    name: "{{feature-name}}",
    description: "{{feature_description}}",
    layers: ["{{selected_layers}}"],
    mode: "{{development_mode}}",
    started_at: "{{ISO timestamp}}"
  }
})
```

## Phase 2: Behavioral Specs (MANDATORY)

Before ANY code is written, create behavioral specs. This prevents the "tests mirror bugs" problem.

### Register specs task with HexFlo:

```tool
mcp__hex__hex_hexflo_task_create({
  title: "Write behavioral specs for {{feature-name}}",
  assignee: "behavioral-spec-writer",
  metadata: { phase: "specs", feature: "{{feature-name}}", tier: 0 }
})
```

### Spawn the behavioral-spec-writer agent:

```
Agent({
  subagent_type: "general-purpose",
  mode: "bypassPermissions",
  prompt: `You are the behavioral-spec-writer agent for a hex project.

Feature: {{feature_description}}
Project root: {{cwd}}

Instructions:
1. Read src/core/ports/index.ts to understand existing contracts
2. Read src/core/domain/ to understand existing types
3. Write behavioral specs in Given/When/Then format
4. Include negative specs (what should NOT happen)
5. Document any coordinate systems, sign conventions, or domain conventions
6. Save specs to docs/specs/{{feature-name}}.json

Spec format per entry:
{
  "category": "string",
  "description": "human-readable description",
  "given": "precondition",
  "when": "action or event",
  "then": "expected outcome",
  "negative_spec": false,
  "domain_conventions": {}
}

Write at least 5 specs covering: happy path, error cases, edge cases, and at least 1 negative spec.`
})
```

Wait for specs to complete. Then mark task done:

```tool
mcp__hex__hex_hexflo_task_complete({
  task_id: "{{specs_task_id}}",
  result: "Wrote {{N}} behavioral specs to docs/specs/{{feature-name}}.json"
})
```

**IMPORTANT**: `feature-workflow.sh setup` now **enforces** spec existence. The setup command will exit with an error if `docs/specs/<feature>.json` does not exist. This is intentional -- specs must be written before worktrees are created. For emergency hotfixes only, pass `--skip-specs` to bypass this check:

```bash
./scripts/feature-workflow.sh setup {{feature-name}} --skip-specs
```

## Phase 3: Planning — Decompose into Adapter-Bounded Tasks

### Register planning task with HexFlo:

```tool
mcp__hex__hex_hexflo_task_create({
  title: "Decompose {{feature-name}} into adapter-bounded tasks",
  assignee: "planner",
  metadata: { phase: "plan", feature: "{{feature-name}}", tier: 0 }
})
```

### Spawn the planner agent:

```
Agent({
  subagent_type: "general-purpose",
  mode: "bypassPermissions",
  prompt: `You are the planner agent for a hex project.

Feature: {{feature_description}}
Behavioral specs: docs/specs/{{feature-name}}.json
Project root: {{cwd}}

Instructions:
1. Read the behavioral specs
2. Read src/core/ports/index.ts for existing port interfaces
3. Read src/core/domain/ for existing domain types
4. Decompose the feature into adapter-bounded tasks
5. Each task maps to exactly one adapter boundary
6. Order tasks by dependency:
   - Domain/port changes FIRST (other tasks depend on these)
   - Secondary adapters NEXT
   - Primary adapters NEXT
   - Integration tests LAST
7. Max 8 parallel tasks
8. Write workplan to docs/workplans/feat-{{feature-name}}.json

Workplan schema:
{
  "id": "feat-{{feature-name}}",
  "title": "Feature: {{feature_description}}",
  "specs": "docs/specs/{{feature-name}}.json",
  "steps": [
    {
      "id": "step-1",
      "description": "what to do",
      "layer": "domain|ports|usecases|adapters/primary|adapters/secondary",
      "adapter": "adapter-name (if applicable)",
      "port": "IPortName (if applicable)",
      "dependencies": [],
      "worktree_branch": "feat/{{feature-name}}/{{adapter-name}}",
      "done_condition": "compile + lint + test pass"
    }
  ]
}`
})
```

Wait for workplan. Then mark task done and register all coding tasks:

```tool
mcp__hex__hex_hexflo_task_complete({
  task_id: "{{plan_task_id}}",
  result: "Workplan with {{N}} steps written to docs/workplans/feat-{{feature-name}}.json"
})
```

### Register each workplan step as a HexFlo task:

For EACH step in the workplan:

```tool
mcp__hex__hex_hexflo_task_create({
  title: "{{step.description}}",
  assignee: "hex-coder",
  metadata: {
    phase: "code",
    feature: "{{feature-name}}",
    step_id: "{{step.id}}",
    adapter: "{{step.adapter}}",
    port: "{{step.port}}",
    layer: "{{step.layer}}",
    tier: {{tier_number}},
    worktree_branch: "{{step.worktree_branch}}",
    dependencies: ["{{step.dependencies}}"]
  }
})
```

Store workplan reference in HexFlo memory:

```tool
mcp__hex__hex_hexflo_memory_store({
  key: "feature/{{feature-name}}/workplan",
  value: {
    workplan_path: "docs/workplans/feat-{{feature-name}}.json",
    task_ids: { "step-1": "hexflo-task-id-1", "step-2": "hexflo-task-id-2" },
    total_steps: {{N}},
    completed_steps: 0
  }
})
```

## Phase 4: Worktree Setup and Parallel Coding

### 4a. Create worktrees for each task

Run `scripts/feature-workflow.sh setup {{feature-name}}` to create worktrees from the workplan.

If the script doesn't exist yet, create worktrees manually:

```bash
# For each step in the workplan:
git worktree add ../hex-feat-{{feature-name}}-{{adapter}} feat/{{feature-name}}/{{adapter}}
```

### 4b. Spawn hex-coder agents in parallel

CRITICAL: Spawn ALL independent agents in a SINGLE message (parallel tool calls).
CRITICAL: Use mode=bypassPermissions for background agents that write files.

For each task that has no unfinished dependencies, spawn a hex-coder:

```
Agent({
  subagent_type: "coder",
  mode: "bypassPermissions",
  run_in_background: true,
  prompt: `You are a hex-coder agent.

Feature: {{feature_description}}
Task: {{step.description}}
Adapter: {{step.adapter}}
Port: {{step.port}}
Worktree: ../hex-feat-{{feature-name}}-{{step.adapter}}
Behavioral specs: docs/specs/{{feature-name}}.json
HexFlo task ID: {{hexflo_task_id}}

Instructions:
1. cd to the worktree directory
2. Read the port interface from src/core/ports/index.ts
3. Read the behavioral specs relevant to your adapter
4. TDD Red: Write failing tests in tests/unit/{{adapter}}.test.ts
5. TDD Green: Implement adapter in src/adapters/{{layer}}/{{adapter}}.ts
6. TDD Refactor: Clean up, extract helpers if needed
7. Run: bun run check && bun test && bun run lint
8. Commit changes with message: feat({{adapter}}): implement {{port}} for {{feature-name}}
9. Report the commit hash in your response

Constraints:
- NEVER import from other adapters
- ONLY import from core/ports and core/domain
- Use .js extensions in all relative imports
- Max 500 lines per file`
})
```

### 4c. On agent completion — mark HexFlo task done

When each hex-coder agent completes, immediately mark its HexFlo task:

```tool
mcp__hex__hex_hexflo_task_complete({
  task_id: "{{hexflo_task_id}}",
  result: "feat({{adapter}}): implement {{port}} — commit {{hash}}"
})
```

Update progress in HexFlo memory:

```tool
mcp__hex__hex_hexflo_memory_store({
  key: "feature/{{feature-name}}/progress",
  value: {
    completed: ["step-1", "step-3"],
    in_progress: ["step-2"],
    pending: ["step-4", "step-5"],
    current_tier: 1,
    last_updated: "{{ISO timestamp}}"
  }
})
```

### 4d. Handle dependency tiers

Tasks execute in tiers based on dependencies:
- **Tier 0**: Domain + port changes (no dependencies)
- **Tier 1**: Secondary adapters (depend on ports)
- **Tier 2**: Primary adapters (depend on ports)
- **Tier 3**: Use case changes (depend on ports)
- **Tier 4**: Composition root wiring
- **Tier 5**: Integration tests (depend on everything)

After all agents in a tier complete, check HexFlo task list for next tier:

```tool
mcp__hex__hex_hexflo_task_list()
```

Spawn agents for the next tier's pending tasks.

## Phase 5: Validation (BLOCKING GATE)

Register validation task:

```tool
mcp__hex__hex_hexflo_task_create({
  title: "Validate {{feature-name}} against behavioral specs",
  assignee: "validation-judge",
  metadata: { phase: "validate", feature: "{{feature-name}}", blocking: true }
})
```

Spawn the validation judge:

```
Agent({
  subagent_type: "general-purpose",
  mode: "bypassPermissions",
  prompt: `You are the validation-judge agent.

Feature: {{feature_description}}
Behavioral specs: docs/specs/{{feature-name}}.json
Project root: {{cwd}}

Instructions:
1. Read the behavioral specs
2. For EACH spec, verify the implementation satisfies it
3. Run: bun run build
4. Run: bun test
5. Run: bunx hex analyze . (architecture boundary check)
6. Generate property-based tests for critical invariants
7. Check that the app is actually runnable (not just "tests pass")

Output a verdict:
{
  "verdict": "PASS" | "FAIL",
  "specs_checked": number,
  "specs_passed": number,
  "specs_failed": [{ "spec": "...", "reason": "..." }],
  "architecture_violations": [],
  "recommendations": []
}

If FAIL: list exactly what needs to change for each failing spec.`
})
```

Mark validation result:

```tool
mcp__hex__hex_hexflo_task_complete({
  task_id: "{{validate_task_id}}",
  result: "Verdict: {{PASS|FAIL}} — {{specs_passed}}/{{specs_checked}} specs passed"
})
```

If verdict is FAIL, iterate: fix the issues and re-validate (max 2 retries).
If verdict is PASS, proceed to Phase 6.

## Phase 6: Integration and Merge

Register integration task:

```tool
mcp__hex__hex_hexflo_task_create({
  title: "Merge and integrate {{feature-name}} worktrees",
  assignee: "integrator",
  metadata: { phase: "integrate", feature: "{{feature-name}}" }
})
```

### 6a. Merge worktrees in dependency order

```bash
# Run the feature-workflow script:
./scripts/feature-workflow.sh merge {{feature-name}}

# Or manually in dependency order:
git merge feat/{{feature-name}}/domain --no-ff
git merge feat/{{feature-name}}/ports --no-ff
git merge feat/{{feature-name}}/{{secondary-adapter}} --no-ff
git merge feat/{{feature-name}}/{{primary-adapter}} --no-ff
git merge feat/{{feature-name}}/integration --no-ff
```

### 6b. Run full test suite on merged result

```bash
bun run check && bun test && bun run lint && bunx hex analyze .
```

### 6c. Clean up worktrees

```bash
./scripts/feature-workflow.sh cleanup {{feature-name}}
```

Mark integration done:

```tool
mcp__hex__hex_hexflo_task_complete({
  task_id: "{{integrate_task_id}}",
  result: "Feature {{feature-name}} merged to main — commit {{final_hash}}"
})
```

## Phase 7: Finalize

1. Update composition-root.ts if new adapters need wiring
2. Run final `bun run build` to verify clean build
3. Commit with: `feat: {{feature-name}} — {{one-line summary}}`

Store final report in HexFlo memory:

```tool
mcp__hex__hex_hexflo_memory_store({
  key: "feature/{{feature-name}}/report",
  value: {
    status: "complete",
    verdict: "PASS",
    specs_file: "docs/specs/{{feature-name}}.json",
    workplan_file: "docs/workplans/feat-{{feature-name}}.json",
    tasks_completed: {{N}},
    files_changed: ["..."],
    tests_added: {{N}},
    integration_commit: "{{hash}}",
    completed_at: "{{ISO timestamp}}"
  }
})
```

Report completion with summary of files changed, tests added, specs validated.

## Quick Reference

| Phase | Agent | HexFlo Action | Gate |
|-------|-------|-------------|------|
| Init | — | `swarm_init` + `task_list` (reconcile) | Swarm active |
| Specs | behavioral-spec-writer | `task_create` → `task_complete` | Specs exist |
| Plan | planner | `task_create` → `task_complete` + register all steps | Workplan valid |
| Code | hex-coder (x N) | `task_complete` per agent + `memory_store` progress | compile + lint + test |
| Validate | validation-judge | `task_create` → `task_complete` with verdict | PASS verdict |
| Integrate | integrator | `task_create` → `task_complete` with commit hash | Full suite passes |
| Finalize | — | `memory_store` final report | Build clean |

## Worktree Branch Naming

```
feat/{{feature-name}}/domain          # Domain type changes
feat/{{feature-name}}/ports           # Port interface changes
feat/{{feature-name}}/{{adapter}}     # Per-adapter work
feat/{{feature-name}}/integration     # Integration tests
```

## Session Continuity

If a session ends mid-feature, the next session can resume by:

```tool
mcp__hex__hex_hexflo_memory_retrieve({ key: "feature/{{feature-name}}/progress" })
mcp__hex__hex_hexflo_task_list()
```

Cross-reference HexFlo task status against `git log --oneline -10` to reconcile completed work. Resume from the next incomplete tier.

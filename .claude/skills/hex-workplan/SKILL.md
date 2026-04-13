---
name: hex-workplan
description: Create, validate, and manage hex workplans and behavioral specs. Use when the user asks to "create workplan", "write a plan", "plan this feature", "workplan format", "spec format", "behavioral specs", or "decompose into tasks".
---

# Hex Workplan — Feature Planning and Spec Authoring

Workplans are the structured task graphs that drive hex feature development. They decompose features into adapter-bounded steps with dependency ordering, tier assignment, and spec traceability. HexFlo executes workplans by dispatching agents per step.

## Two Artifacts, One Pipeline

Every hex feature requires **two** JSON artifacts before code begins:

1. **Behavioral Specs** → `docs/specs/<feature>.json`
2. **Workplan** → `docs/workplans/feat-<feature>.json`

Specs come first (enforced by the hex-specs-required hook). The workplan references specs by ID so every coding task is traceable to a behavioral expectation.

---

## Behavioral Spec Format

Location: `docs/specs/<feature-name>.json`

```json
{
  "feature": "<feature-name>",
  "description": "One-line summary of what this feature does",
  "specs": [
    {
      "id": "S01",
      "category": "CategoryName",
      "description": "Human-readable description of the expected behavior",
      "given": "Precondition — the state of the system before the action",
      "when": "Action or event that triggers the behavior",
      "then": "Expected outcome — the observable result",
      "negative_spec": false,
      "domain_conventions": {
        "key": "Optional domain-specific conventions (encryption, coordinate systems, etc.)"
      }
    }
  ]
}
```

### Spec Rules

- **Minimum 3 specs** per feature (happy path, error case, edge case)
- **At least 1 negative spec** (`negative_spec: true`) — what should NOT happen
- **IDs are sequential**: S01, S02, S03...
- **Categories group related behaviors**: e.g., "LocalVaultAdapter", "CachingLayer", "CLI"
- **`given/when/then` must be implementation-agnostic** — describe behavior, not code
- **`domain_conventions`** captures sign conventions, encryption params, coordinate systems, or any domain knowledge the coder needs

### Spec Categories

Use these standard categories where applicable:

| Category | For |
|----------|-----|
| HappyPath | Normal successful operations |
| ErrorHandling | Expected error conditions |
| EdgeCase | Boundary values, empty inputs, limits |
| Security | Auth, encryption, injection prevention |
| Performance | Latency, throughput, resource constraints |
| Integration | Cross-adapter interaction behavior |

### Example Behavioral Spec

```json
{
  "feature": "user-auth",
  "description": "JWT-based user authentication with refresh tokens",
  "specs": [
    {
      "id": "S01",
      "category": "HappyPath",
      "description": "Valid credentials return a JWT token pair",
      "given": "A registered user with email 'test@example.com' and valid password",
      "when": "authenticate(email, password) is called",
      "then": "Returns { accessToken: <JWT>, refreshToken: <UUID>, expiresIn: 3600 }",
      "negative_spec": false,
      "domain_conventions": { "jwt_algo": "HS256", "access_ttl_seconds": 3600 }
    },
    {
      "id": "S02",
      "category": "Security",
      "description": "Invalid password is rejected without leaking which field failed",
      "given": "A registered user with email 'test@example.com'",
      "when": "authenticate(email, 'wrong-password') is called",
      "then": "Returns error 'Invalid credentials' (no mention of 'password' specifically)",
      "negative_spec": true,
      "domain_conventions": { "security": "Uniform error message prevents username enumeration" }
    },
    {
      "id": "S03",
      "category": "EdgeCase",
      "description": "Expired refresh token cannot generate new access token",
      "given": "A refresh token that was issued 31 days ago (TTL is 30 days)",
      "when": "refreshAccessToken(expiredToken) is called",
      "then": "Returns error 'Refresh token expired' and does not issue new tokens",
      "negative_spec": true
    }
  ]
}
```

---

## Workplan Format

Location: `docs/workplans/feat-<feature-name>.json`

```json
{
  "id": "feat-<feature-name>",
  "title": "Feature: <one-line description>",
  "specs": "docs/specs/<feature-name>.json",
  "adr": "ADR-NNN",
  "created": "YYYY-MM-DD",
  "status": "planned | in_progress | complete",
  "topology": "hierarchical | mesh | pipeline",
  "budget": "~NNNNN tokens",
  "prior_work": {
    "done": [
      "List of files/components already completed (optional)"
    ]
  },
  "steps": [
    {
      "id": "step-1",
      "description": "What to implement in this step",
      "layer": "domain|ports|usecases|adapters/primary|adapters/secondary|integration",
      "adapter": "adapter-name (if applicable)",
      "port": "IPortName (if applicable)",
      "dependencies": [],
      "tier": 0,
      "specs": ["S01", "S02"],
      "worktree_branch": "feat/<feature>/<adapter>",
      "done_condition": "compile + specific test criteria"
    }
  ],
  "dependencies": {
    "cargo": [
      { "name": "crate-name", "version": "1.0", "purpose": "why needed" }
    ],
    "npm": [
      { "name": "package", "version": "^1.0", "purpose": "why needed" }
    ]
  },
  "mergeOrder": [
    "step-1 → step-2 (domain then ports)",
    "step-3, step-4 parallel → step-5 last"
  ],
  "riskRegister": [
    { "risk": "Description of what could go wrong", "impact": "high|medium|low", "mitigation": "How to handle it" }
  ],
  "successCriteria": [
    "Observable outcome that proves the feature works end-to-end"
  ]
}
```

### Workplan Fields

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `id` | string | yes | Unique ID, format: `feat-<feature-name>` |
| `title` | string | yes | Human-readable feature title |
| `specs` | string | yes | Path to behavioral specs file |
| `adr` | string | no | Related ADR (e.g., ADR-027) |
| `created` | string | no | Creation date (YYYY-MM-DD) |
| `status` | string | no | Runtime status: planned, in_progress, complete |
| `topology` | string | no | Swarm topology: hierarchical, mesh, pipeline |
| `budget` | string | no | Estimated token budget |
| `prior_work` | object | no | Already-completed work from previous sessions |
| `prior_work.done` | string[] | no | List of completed files/components |
| `dependencies` | object | no | External dependencies needed (cargo crates, npm packages) |
| `mergeOrder` | string[] | no | Explicit merge ordering instructions |
| `riskRegister` | object[] | no | Risks with impact levels and mitigations |
| `successCriteria` | string[] | no | Observable outcomes that prove the feature works |
| `steps` | array | yes | Ordered list of adapter-bounded tasks |

### Step Fields

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `id` | string | yes | Unique step ID: `step-1`, `step-2`, ... |
| `description` | string | yes | What to implement — specific enough for a hex-coder agent |
| `layer` | string | yes | Hexagonal layer this step targets |
| `adapter` | string | yes* | Adapter name (required for adapter-layer steps) |
| `port` | string | no | Port interface being implemented |
| `dependencies` | string[] | yes | Step IDs that must complete before this one |
| `tier` | integer | yes | Execution tier (0-5), determines parallelism |
| `specs` | string[] | yes | Behavioral spec IDs this step satisfies |
| `worktree_branch` | string | no | Git worktree branch name |
| `done_condition` | string | yes | Specific criteria for marking step complete |
| `status` | string | no | Runtime status: pending, in_progress, completed, failed |
| `commit_hash` | string | no | Git commit hash when completed |

### Tier Ordering

Tiers determine which steps can run in parallel. Lower tiers must complete before higher tiers start:

| Tier | Layer | Depends On | Parallelism |
|------|-------|------------|-------------|
| 0 | Domain types + Port interfaces | Nothing | Sequential (foundational) |
| 1 | Secondary adapters | Tier 0 | Parallel within tier |
| 2 | Primary adapters | Tier 0 | Parallel within tier |
| 3 | Use cases | Tiers 0-2 | Sequential (orchestrates adapters) |
| 4 | Composition root | Tiers 0-3 | Sequential (wiring) |
| 5 | Integration tests | Everything | Sequential (validates all) |

### Dependency Rules

- Every step must list its dependencies explicitly (no implicit ordering)
- Dependencies must respect tier ordering (a tier-1 step cannot depend on tier-3)
- Port interface changes (tier 0) are implicit dependencies for all adapter steps
- Integration tests (tier 5) depend on all steps they exercise
- Maximum 8 parallel steps per tier (matches worktree limit)

---

## Creating a Workplan

### Step 1: Write Behavioral Specs First

```tool
Agent({
  subagent_type: "general-purpose",
  mode: "bypassPermissions",
  prompt: "You are the behavioral-spec-writer agent. Feature: <description>. Write specs to docs/specs/<feature>.json"
})
```

### Step 2: Decompose into Steps

For each behavioral spec:
1. Identify which adapter boundary it touches
2. Group specs by adapter → each group becomes a step
3. Assign tiers based on layer hierarchy
4. Set dependencies based on which steps need outputs from others

### Step 3: Write the Workplan

Save to `docs/workplans/feat-<feature>.json`

### Step 4: Register with HexFlo

For each step, create a HexFlo task:
```tool
mcp__hex__hex_hexflo_task_create({
  swarm_id: "<swarm-id>",
  title: "<step.description>",
  assignee: "hex-coder",
  metadata: {
    phase: "code",
    feature: "<feature>",
    step_id: "<step.id>",
    tier: <step.tier>,
    adapter: "<step.adapter>",
    port: "<step.port>"
  }
})
```

### Step 5: Store Workplan Reference in Memory

```tool
mcp__hex__hex_hexflo_memory_store({
  key: "feature/<feature>/workplan",
  value: JSON.stringify({
    workplan_path: "docs/workplans/feat-<feature>.json",
    total_steps: N,
    completed_steps: 0
  })
})
```

---

## Validating a Workplan

Before execution, validate:

1. **Spec traceability**: Every step references at least one spec ID
2. **Spec coverage**: Every spec ID is referenced by at least one step
3. **Tier consistency**: Dependencies respect tier ordering
4. **No cycles**: Dependency graph is a DAG
5. **Layer correctness**: Each step's `layer` matches the adapter it targets
6. **Done conditions**: Every step has a testable done_condition
7. **Build gates** (REQUIRED for delete/restructure steps): Every phase that deletes or modifies artifacts MUST include `cargo check --workspace` as a blocking gate before the next phase can start
8. **Consumer dependency map**: For any step that deletes files/modules, verify all consumers across ALL workspace crates are identified with corresponding fix steps
9. **Adversarial review gate**: For migrations with >5 deletion steps, include an explicit adversarial-reviewer step after the final phase

```bash
# Validate workplan structure
hex plan status feat-<feature>.json
```

---

## Workplan Execution Flow

```
hex plan execute docs/workplans/feat-<feature>.json
  │
  ├─ Tier 0: domain + ports (sequential)
  │   └─ Wait for completion → verify compile
  │
  ├─ Tier 1: secondary adapters (parallel)
  │   ├─ hex-coder → git-adapter worktree
  │   ├─ hex-coder → fs-adapter worktree
  │   └─ hex-coder → llm-adapter worktree
  │       └─ Wait for all → verify tests
  │
  ├─ Tier 2: primary adapters (parallel)
  │   ├─ hex-coder → cli-adapter worktree
  │   └─ hex-coder → http-adapter worktree
  │       └─ Wait for all → verify tests
  │
  ├─ Tier 3: usecases (sequential)
  │   └─ Wait → verify compile + tests
  │
  ├─ Tier 4: composition root (sequential)
  │   └─ Wait → verify full build
  │
  └─ Tier 5: integration tests
      └─ validation-judge → PASS/FAIL verdict
```

---

## Workplan Lifecycle Commands

| Command | What it does |
|---------|-------------|
| `/hex-workplan create <feature>` | Create workplan from behavioral specs |
| `/hex-workplan validate <file>` | Validate workplan structure and traceability |
| `hex plan list` | List all workplans with progress |
| `hex plan status <file>` | Show detailed workplan status |
| `hex plan execute <file>` | Start workplan execution via HexFlo |
| `hex plan pause` | Pause active execution |
| `hex plan resume` | Resume paused execution |
| `hex plan report <id>` | Execution report with agent performance |
| `hex plan history` | All past executions |

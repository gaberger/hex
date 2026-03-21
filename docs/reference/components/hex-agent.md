# Component: hex-agent

## One-Line Summary

Architecture enforcement runtime — must be present locally or remotely on any system running hex development agents; enforces hexagonal architecture through skills, hooks, ADRs, workplans, HexFlo dispatchers, and agent definitions.

> **Not to be confused with** the hexagonal architecture concept of "adapter." hex-agent is the *agent runtime* — the software that makes AI development agents produce architecture-compliant code. It is named "agent" because it is the runtime environment for hex's AI agents.

## Key Facts

- Must always be present on any system running hex development agents
- Can run locally or on remote compute nodes
- Connects to SpacetimeDB for coordination state
- Uses hex-nexus for filesystem and git operations
- Uses hex's own agent system and HexFlo dispatchers for building software
- Model-agnostic — works with any LLM provider

## Enforcement Mechanisms

### 1. Skills (Slash Commands)

Skills are Markdown-defined prompts that guide AI agents through hex-compliant workflows. They are the primary user-facing interface for architecture-aware code generation.

| Skill | Trigger | Purpose |
|:------|:--------|:--------|
| `/hex-feature-dev` | Start feature development | Full lifecycle: specs → plan → worktrees → code → validate → merge |
| `/hex-scaffold` | Scaffold new project | Creates hex directory structure with proper layer separation |
| `/hex-generate` | Generate adapter code | Generates code within a single adapter boundary |
| `/hex-summarize` | AST summaries | Token-efficient L0–L3 code summaries via tree-sitter |
| `/hex-analyze-arch` | Architecture check | Static analysis for boundary violations, dead code, cycles |
| `/hex-analyze-deps` | Dependency analysis | Tech stack recommendation and dependency mapping |
| `/hex-validate` | Post-build validation | Behavioral spec assertions + property tests + smoke tests |

**Locations:** `.claude/skills/` (IDE integration), `skills/` (shipped in npm package)

### 2. Hooks (Pre/Post Operation Triggers)

Hooks execute automatically before or after operations to validate, format, and learn.

| Hook | When | What It Does |
|:-----|:-----|:-------------|
| `pre-edit` | Before file write | Validate syntax, check boundaries, backup file |
| `post-edit` | After file write | Auto-format, validate boundaries, train patterns |
| `pre-task` | Before task starts | Load memory, auto-spawn agents, optimize topology |
| `post-task` | After task completes | Analyze performance, store decisions, export learnings |
| `session-start` | Session begins | Initialize context, load previous session state |
| `session-end` | Session ends | Persist state, export metrics, generate summary |

**Configuration:** `.claude/settings.json` under `hooks` key

### 3. ADRs (Architecture Decision Records)

37 ADRs in `docs/adrs/` document design decisions. hex-agent enforces these through:
- `architecture-enforcer` WASM module validates boundary rules server-side
- `hex analyze` checks ADR compliance locally
- `adr-reviewer` agent reviews PRs against active ADRs

### 4. Workplans

Structured task decomposition stored in `docs/workplans/`. A workplan breaks a feature into adapter-bounded steps organized by dependency tier:

```
Tier 0: domain + ports (no dependencies)
Tier 1: secondary adapters (depends on Tier 0)
Tier 2: primary adapters (depends on Tier 0)
Tier 3: use cases (depends on Tiers 0–2)
Tier 4: composition root (depends on Tiers 0–3)
Tier 5: integration tests (depends on everything)
```

### 5. HexFlo Dispatchers

Native Rust coordination for multi-agent swarm execution. Dispatchers manage:
- Swarm initialization with topology selection
- Task creation and assignment
- Agent lifecycle (register, heartbeat, stale/dead detection)
- Memory coordination (store, retrieve, search)

### 6. Agent Definitions

14 YAML-defined agent roles with specific boundaries and constraints:

| Agent | Role | Boundary |
|:------|:-----|:---------|
| `feature-developer` | Orchestrates full feature lifecycle | All layers |
| `planner` | Decomposes requirements into tasks | Read-only analysis |
| `hex-coder` | Codes within one adapter boundary (TDD) | Single adapter |
| `integrator` | Merges worktrees, integration tests | Cross-adapter merge |
| `swarm-coordinator` | Orchestrates lifecycle via HexFlo | Coordination only |
| `behavioral-spec-writer` | Writes acceptance specs | Specs only |
| `validation-judge` | Post-build validation (**BLOCKING**) | Read-only verification |
| `dead-code-analyzer` | Finds dead exports + violations | Read-only analysis |
| `scaffold-validator` | Ensures projects are runnable | Project structure |
| `dependency-analyst` | Recommends tech stack | Read-only analysis |
| `status-monitor` | Swarm progress monitoring | Read-only monitoring |
| `adr-reviewer` | Reviews ADR compliance | ADR files |
| `rust-refactorer` | Rust-specific refactoring | Rust files |
| `dev-tracker` | Development activity tracking | Activity logs |

**Locations:** `agents/` (shipped in npm), `.claude/agents/` (IDE integration)

## Depends On

- **SpacetimeDB** — coordination state, agent registry, enforcement rules
- **hex-nexus** — filesystem operations, git, architecture analysis
- **hex-core** — shared domain types

## Depended On By

- Any system running hex development agents (required component)

## Related ADRs

- ADR-001: Hexagonal Architecture (foundational rules)
- ADR-014: Dependency Injection via Deps Pattern (testing strategy)
- ADR-027: HexFlo Swarm Coordination
- ADR-045: ADR Compliance Enforcement

# hex — Hexagonal Architecture for LLM-Driven Development

## What This Project Is

hex is a **harness** — a framework + CLI tool that gets **installed into target projects** for AI-driven development using hexagonal architecture (ports & adapters). This repo is NOT an application. It is the installable framework that scaffolds and manages other projects.

**Critical**: Everything in this repo (settings, hooks, statuslines, agents, skills) exists to be instantiated INTO a target project via `hex setup` or `hex scaffold`. The `examples/` directory contains sample target projects that use hex as an installed dependency. When working on examples, you are testing hex as a consumer would use it — the example IS the project, hex is the tool.

hex provides token-efficient code summaries via tree-sitter, swarm coordination via ruflo, and a specs-first development pipeline.

## Behavioral Rules

- Do what has been asked; nothing more, nothing less
- ALWAYS read a file before editing it
- NEVER save files to the root folder — use the directories below
- NEVER commit secrets, credentials, or .env files
- ALWAYS run `bun test` after making code changes
- ALWAYS run `bun run build` before committing

## Hexagonal Architecture Rules (ENFORCED)

These rules are checked by `hex analyze .` and the dead-code-analyzer agent:

1. **domain/** must only import from **domain/** (value-objects, entities)
2. **ports/** may import from **domain/** (for value types) but nothing else
3. **usecases/** may import from **domain/** and **ports/** only
4. **adapters/primary/** may import from **ports/** only
5. **adapters/secondary/** may import from **ports/** only
6. **adapters must NEVER import other adapters** (cross-adapter coupling)
7. **composition-root.ts** is the ONLY file that imports from adapters — this is by design
8. All relative imports MUST use `.js` extensions (NodeNext module resolution)

## File Organization

```
src/
  core/
    domain/          # Pure business logic, zero external deps
      value-objects.ts  # Shared types (Language, ASTSummary, etc.)
      entities.ts       # Domain events, QualityScore, FeedbackLoop, TaskGraph
    ports/           # Typed interfaces — contracts between layers
    usecases/        # Application logic composing ports
  adapters/
    primary/         # Driving adapters (CLI, MCP, dashboard, browser input)
    secondary/       # Driven adapters (FS, Git, LLM, tree-sitter, ruflo)
  infrastructure/    # Cross-cutting (tree-sitter queries)
  composition-root.ts  # Wires adapters → ports (single DI point)
  cli.ts              # CLI entry point
  index.ts            # Library public API
tests/
  unit/              # London-school mock-first tests
  integration/       # Real adapter tests
examples/            # Example applications (Flappy Bird, etc.)
docs/
  architecture/      # Architecture specs and research
  adrs/              # Architecture Decision Records
  analysis/          # Adversarial review reports
config/              # Language configs, tree-sitter settings
scripts/             # Build and setup scripts
.claude/
  skills/            # Claude Code skills (.md) — /hex-scaffold, /hex-generate, etc.
  agents/hex/        # Agent definitions (.yml) — planner, hex-coder, etc.
agents/              # Agent source definitions (YAML, shipped in npm package)
skills/              # Skill source definitions (.md, shipped in npm package)
```

## Build & Test

```bash
bun run build        # Bundle CLI + library to dist/
bun test             # Run all tests (unit + property + smoke)
bun run check        # TypeScript type check (no emit)
hex analyze .        # Architecture health check
hex setup            # Install grammars + skills + agents
```

## Development Pipeline (Specs-First)

When building new features or example applications, follow this order:

1. **Specify** — Write behavioral specs BEFORE code (what "correct" looks like)
2. **Build** — Generate code following hex architecture rules
3. **Test** — Unit tests + property tests + smoke tests (3 levels)
4. **Validate** — Run `hex analyze` + validation judge
5. **Ship** — README + start scripts + commit

## Feature Development Workflow

In hex architecture, a "feature" is NOT a vertical slice. It decomposes inside-out across layers, with each adapter boundary getting its own git worktree for isolation.

### How to Start a Feature

Use `/hex-feature-dev` or run the shell script directly:

```bash
# Interactive (via Claude Code skill)
/hex-feature-dev

# Shell script for worktree lifecycle
./scripts/feature-workflow.sh setup <feature-name>     # Create worktrees from workplan
./scripts/feature-workflow.sh status <feature-name>     # Show progress
./scripts/feature-workflow.sh merge <feature-name>      # Merge in dependency order
./scripts/feature-workflow.sh cleanup <feature-name>    # Remove worktrees + branches
./scripts/feature-workflow.sh list                      # List all feature worktrees
./scripts/feature-workflow.sh stale                     # Find abandoned worktrees
```

### Feature Lifecycle (7 Phases)

```
Phase 1: SPECS       behavioral-spec-writer → docs/specs/<feature>.json
Phase 2: PLAN        planner → docs/workplans/feat-<feature>.json
Phase 3: WORKTREES   feature-workflow.sh setup → one worktree per adapter
Phase 4: CODE        hex-coder agents (parallel, TDD) in isolated worktrees
Phase 5: VALIDATE    validation-judge → PASS/FAIL verdict (BLOCKING)
Phase 6: INTEGRATE   merge worktrees in dependency order → run full suite
Phase 7: FINALIZE    cleanup worktrees, commit, report
```

### Worktree Conventions

- **Naming**: `feat/<feature-name>/<layer-or-adapter>`
- **Max concurrent**: 8 worktrees
- **Merge order**: domain → ports → secondary adapters → primary adapters → usecases → integration
- **Cleanup**: Always remove worktrees after successful merge
- **Stale detection**: Worktrees older than 24h with no commits are flagged

### Dependency Tiers (What Runs When)

| Tier | Layer | Depends On | Agent |
|------|-------|------------|-------|
| 0 | Domain + Ports | Nothing | hex-coder |
| 1 | Secondary adapters | Tier 0 | hex-coder |
| 2 | Primary adapters | Tier 0 | hex-coder |
| 3 | Use cases + Composition root | Tiers 1-2 | hex-coder |
| 4 | Integration tests | Everything | integrator |

### Development Modes

| Mode | When to Use |
|------|------------|
| **Swarm** (default) | Features spanning 2+ adapters — parallel worktrees |
| **Interactive** | Critical features needing human review at each phase |
| **Single-agent** | Small changes within one adapter boundary |

## Available Skills (Claude Code slash commands)

| Skill | Trigger |
|-------|---------|
| `/hex-feature-dev` | Start feature development with hex decomposition |
| `/hex-scaffold` | Scaffold a new hex project |
| `/hex-generate` | Generate code within an adapter boundary |
| `/hex-summarize` | Token-efficient AST summaries (L0-L3) |
| `/hex-analyze-deps` | Dependency analysis + tech stack recommendation |
| `/hex-analyze-arch` | Architecture health check |
| `/hex-validate` | Post-build semantic validation |

## Available Agents

| Agent | Role |
|-------|------|
| `feature-developer` | Orchestrates full feature lifecycle (specs → code → validate → merge) |
| `planner` | Decomposes requirements into adapter-bounded tasks |
| `hex-coder` | Codes within one adapter with TDD loop |
| `integrator` | Merges worktrees, integration tests |
| `swarm-coordinator` | Orchestrates full lifecycle via ruflo |
| `dependency-analyst` | Recommends tech stack + runtime requirements |
| `dead-code-analyzer` | Finds dead exports + hex boundary violations |
| `scaffold-validator` | Ensures projects are runnable (README, scripts, dev server) |
| `behavioral-spec-writer` | Writes acceptance specs before code generation |
| `validation-judge` | Post-build semantic validation (BLOCKING gate) |
| `status-monitor` | Swarm progress monitoring |

## Key Lessons (from adversarial review)

- **Tests can mirror bugs**: When the same LLM writes code AND tests, tests may encode the LLM's misunderstanding. Use property tests and behavioral specs as independent oracles.
- **Sign conventions matter**: For physics/math domains, document coordinate systems explicitly. `flapStrength` must be NEGATIVE (upward force in screen coords).
- **"It compiles" ≠ "it works"**: Always include runtime validation — can a user actually start the app?
- **Browser TypeScript needs a dev server**: Any project with HTML + TypeScript MUST include Vite (or equivalent).

## Swarm Coordination (ruflo)

ruflo (`@claude-flow/cli`) is a required dependency of hex. Used for:
- Task tracking: `ISwarmPort.createTask/completeTask`
- Agent lifecycle: `ISwarmPort.spawnAgent/terminateAgent`
- Swarm topology: `ISwarmPort.init` (hierarchical/mesh)
- Persistent memory: `ISwarmPort.memoryStore/Retrieve`

```bash
# Always use background agents with bypassPermissions for file writes
Agent tool: { subagent_type: "coder", mode: "bypassPermissions", run_in_background: true }
```

## Security

- `FileSystemAdapter` has path traversal protection via `safePath()`
- `RufloAdapter` uses `execFile` (not `exec`) — no shell injection
- API keys loaded only in `composition-root.ts` from env vars
- Never commit `.env` files — use `.env.example`
- Primary adapters MUST NOT use `innerHTML`/`outerHTML`/`insertAdjacentHTML` with any data that originates outside the domain layer. Use `textContent` or DOM APIs (`createElement`) instead.

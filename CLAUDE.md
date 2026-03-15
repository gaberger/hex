# hex-intf — Hexagonal Architecture for LLM-Driven Development

## What This Project Is

hex-intf is a framework + CLI tool for building software using hexagonal architecture (ports & adapters) optimized for AI agent code generation. It provides token-efficient code summaries via tree-sitter, swarm coordination via ruflo, and a specs-first development pipeline.

## Behavioral Rules

- Do what has been asked; nothing more, nothing less
- ALWAYS read a file before editing it
- NEVER save files to the root folder — use the directories below
- NEVER commit secrets, credentials, or .env files
- ALWAYS run `bun test` after making code changes
- ALWAYS run `bun run build` before committing

## Hexagonal Architecture Rules (ENFORCED)

These rules are checked by `hex-intf analyze .` and the dead-code-analyzer agent:

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
  agents/hex-intf/   # Agent definitions (.yml) — planner, hex-coder, etc.
agents/              # Agent source definitions (YAML, shipped in npm package)
skills/              # Skill source definitions (.md, shipped in npm package)
```

## Build & Test

```bash
bun run build        # Bundle CLI + library to dist/
bun test             # Run all tests (unit + property + smoke)
bun run check        # TypeScript type check (no emit)
hex-intf analyze .   # Architecture health check
hex-intf setup       # Install grammars + skills + agents
```

## Development Pipeline (Specs-First)

When building new features or example applications, follow this order:

1. **Specify** — Write behavioral specs BEFORE code (what "correct" looks like)
2. **Build** — Generate code following hex architecture rules
3. **Test** — Unit tests + property tests + smoke tests (3 levels)
4. **Validate** — Run `hex-intf analyze` + validation judge
5. **Ship** — README + start scripts + commit

## Available Skills (Claude Code slash commands)

| Skill | Trigger |
|-------|---------|
| `/hex-scaffold` | Scaffold a new hex project |
| `/hex-generate` | Generate code within an adapter boundary |
| `/hex-summarize` | Token-efficient AST summaries (L0-L3) |
| `/hex-analyze-deps` | Dependency analysis + tech stack recommendation |
| `/hex-analyze-arch` | Architecture health check |
| `/hex-validate` | Post-build semantic validation |

## Available Agents

| Agent | Role |
|-------|------|
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

ruflo (`@claude-flow/cli`) is a required dependency. Used for:
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

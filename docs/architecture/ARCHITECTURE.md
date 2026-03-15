# hex-intf: Hexagonal Architecture for LLM-Driven Development

## Vision

A reusable, packageable framework that enables AI coding agents to generate, validate, and iterate on quality code using hexagonal architecture (ports & adapters) across TypeScript, Go, and Rust — with token-efficient structural communication via tree-sitter AST summaries.

## Core Principles

1. **Hexagonal Architecture (Ports & Adapters)** — All business logic lives in the core domain, accessed only through typed port interfaces. Adapters are swappable and independently testable.
2. **Token Efficiency** — Tree-sitter extracts structural summaries (signatures, types, dependencies) that compress files to ~10% of original token count for LLM context.
3. **Static Typing as Guardrail** — TypeScript, Go, and Rust all provide compile-time error detection. Linters catch LLM-generated defects before test execution.
4. **Fast Feedback Loops** — Compile → Lint → Test in seconds. Each cycle gives the LLM structured error output to self-correct.
5. **Bounded Context per Adapter** — An LLM agent works on one adapter at a time. Port interfaces define the contract; the agent never needs full codebase context.
6. **Swarm Coordination** — Ruflo orchestrates parallel agents across git worktrees, each working on isolated adapters with merge-back via PR.
7. **Dogfooding (ADR-008)** — hex-intf is built using its own hexagonal patterns. The framework's own code serves as both the product and the primary test case. Domain events drive all cross-cutting concerns (notifications, logging, metrics) through the `IEventBusPort`, ensuring adapters never couple to each other.

### Dependency Direction (Enforced)

```
  adapters/primary/ ──▶ ports/ ◀── adapters/secondary/
                          │
                     usecases/
                          │
                      domain/
```

- Domain imports nothing external
- Ports define interfaces only (no implementations)
- Use cases compose ports
- Adapters implement ports and may import external libraries
- **No adapter may import another adapter** — cross-cutting goes through domain events

---

## Architecture Overview

```
┌─────────────────────────────────────────────────────┐
│                   Primary Adapters                   │
│  (CLI, HTTP API, LLM Agent Interface, REPL)         │
├──────────────┬──────────────────┬───────────────────┤
│              │   PORT LAYER     │                    │
│              │  (Typed Ifaces)  │                    │
│              ├──────────────────┤                    │
│              │                  │                    │
│              │   DOMAIN CORE    │                    │
│              │  (Use Cases,     │                    │
│              │   Entities,      │                    │
│              │   Value Objects)  │                    │
│              │                  │                    │
│              ├──────────────────┤                    │
│              │   PORT LAYER     │                    │
│              │  (Typed Ifaces)  │                    │
├──────────────┴──────────────────┴───────────────────┤
│                  Secondary Adapters                  │
│  (DB, FileSystem, Git, TreeSitter, LLM Provider)    │
└─────────────────────────────────────────────────────┘
```

---

## Directory Structure

```
hex-intf/
├── src/
│   ├── core/
│   │   ├── domain/          # Entities, value objects, domain events
│   │   ├── ports/           # Input & output port interfaces
│   │   └── usecases/        # Application services / use case orchestration
│   ├── adapters/
│   │   ├── primary/         # Driving adapters (CLI, HTTP, Agent API)
│   │   └── secondary/       # Driven adapters (DB, FS, Git, LLM providers)
│   └── infrastructure/
│       ├── treesitter/      # AST extraction & token-efficient summaries
│       ├── swarm/           # Ruflo swarm coordination layer
│       └── worktree/        # Git worktree management for parallel agents
├── tests/
│   ├── unit/                # London-school mock-first unit tests
│   └── integration/         # Full adapter integration tests
├── docs/
│   ├── architecture/        # This file and diagrams
│   ├── adrs/                # Architecture Decision Records
│   └── skills/              # Packaged skill definitions
├── config/                  # Language-specific configs, lint rules
├── scripts/                 # Build, test, deploy automation
├── skills/                  # Reusable Claude Code skill definitions
├── agents/                  # Agent type definitions for ruflo
└── examples/                # Example projects using the framework
```

---

## Layer Definitions

### Domain Core (`src/core/domain/`)

Pure business logic with zero external dependencies. Contains:

- **Entities**: `Project`, `CodeUnit`, `TestSuite`, `BuildResult`
- **Value Objects**: `Language` (TS|Go|Rust), `ASTSummary`, `TokenBudget`, `QualityScore`
- **Domain Events**: `CodeGenerated`, `TestsPassed`, `LintFailed`, `BuildSucceeded`

### Ports (`src/core/ports/`)

Typed interfaces that define contracts between core and adapters:

```typescript
// Input Ports (driving)
interface ICodeGenerationPort {
  generateFromSpec(spec: Specification, lang: Language): Promise<CodeUnit>;
  refineFromFeedback(unit: CodeUnit, errors: LintError[]): Promise<CodeUnit>;
}

interface IWorkplanPort {
  createPlan(requirements: string[], lang: Language): Promise<Workplan>;
  executePlan(plan: Workplan): AsyncGenerator<StepResult>;
}

// Output Ports (driven)
interface IASTPort {
  extractSummary(filePath: string): Promise<ASTSummary>;
  diffStructural(before: ASTSummary, after: ASTSummary): StructuralDiff;
}

interface ILLMPort {
  prompt(context: TokenBudget, messages: Message[]): Promise<LLMResponse>;
  streamPrompt(context: TokenBudget, messages: Message[]): AsyncGenerator<string>;
}

interface IBuildPort {
  compile(project: Project): Promise<BuildResult>;
  lint(project: Project): Promise<LintResult>;
  test(project: Project, suite: TestSuite): Promise<TestResult>;
}

interface IWorktreePort {
  create(branchName: string): Promise<WorktreePath>;
  merge(worktree: WorktreePath, target: string): Promise<MergeResult>;
  cleanup(worktree: WorktreePath): Promise<void>;
}
```

### Use Cases (`src/core/usecases/`)

Orchestration logic that composes ports:

- **GenerateCode**: Spec → AST context → LLM prompt → Code → Lint → Fix loop
- **CreateWorkplan**: Requirements → Decompose → Task graph → Assign to agents
- **RunFeedbackLoop**: Code → Compile → Lint → Test → Structured errors → Refine
- **CoordinateSwarm**: Init ruflo → Create worktrees → Assign adapters → Merge

### Primary Adapters (`src/adapters/primary/`)

- **CLIAdapter**: Interactive terminal interface for human developers
- **AgentAdapter**: Programmatic interface for LLM agents (structured JSON I/O)
- **HTTPAdapter**: REST API for external tool integration

### Secondary Adapters (`src/adapters/secondary/`)

- **TreeSitterAdapter**: Wraps tree-sitter for AST extraction per language
- **LLMProviderAdapter**: Anthropic Claude, OpenAI, local models via unified port
- **GitAdapter**: Git operations, worktree management, diff generation
- **FileSystemAdapter**: Read/write with path sanitization
- **BuildToolAdapter**: Language-specific build chains (tsc, go build, cargo)

---

## Token-Efficient LLM Communication

### The Problem

Sending raw source files to an LLM wastes tokens on whitespace, comments, and implementation details. A 500-line TypeScript file uses ~2000 tokens but its *structure* (exports, types, function signatures) needs only ~200 tokens.

### The Solution: Tree-Sitter AST Summaries

```
┌──────────────┐     ┌──────────────┐     ┌──────────────┐
│  Source File  │────▶│  Tree-Sitter │────▶│ AST Summary  │
│  (2000 tok)   │     │   Parser     │     │  (200 tok)   │
└──────────────┘     └──────────────┘     └──────────────┘
```

**AST Summary Format** (token-efficient, LLM-readable):

```
FILE: src/adapters/secondary/git-adapter.ts
LANG: typescript
EXPORTS:
  class GitAdapter implements IGitPort
    + constructor(repoPath: string)
    + async commit(msg: string): Promise<CommitHash>
    + async createWorktree(branch: string): Promise<WorktreePath>
    + async merge(source: string, target: string): Promise<MergeResult>
IMPORTS: [IGitPort, CommitHash, WorktreePath, MergeResult] from ../core/ports
DEPS: simple-git
LINES: 187
```

This gives an LLM agent everything it needs to:
1. Understand what the file does
2. Know the public API contract
3. Decide if it needs to read the full file
4. Generate compatible code in other adapters

### Summary Levels

| Level | Tokens | Contains |
|-------|--------|----------|
| **L0 — Index** | ~5/file | filename + language + line count |
| **L1 — Skeleton** | ~50/file | exports, imports, dependencies |
| **L2 — Signatures** | ~200/file | full type signatures, params, returns |
| **L3 — Full** | ~2000/file | complete source (only when editing) |

Agents start at L1, drill to L2 for related files, and only load L3 for the file they're editing.

---

## Swarm Coordination Model

### Workflow: Spec → Code → Test → Ship

```
1. PLAN PHASE (single agent, Opus)
   ├── Parse requirements
   ├── Generate workplan with task graph
   └── Assign tasks to adapter boundaries

2. EXECUTE PHASE (parallel agents, Sonnet)
   ├── Agent-A: primary/cli-adapter    [worktree: feat/cli]
   ├── Agent-B: secondary/git-adapter  [worktree: feat/git]
   ├── Agent-C: secondary/llm-adapter  [worktree: feat/llm]
   └── Each agent: Generate → Lint → Test → Commit

3. INTEGRATE PHASE (single agent, Opus)
   ├── Merge worktrees to main
   ├── Run integration tests
   ├── Fix cross-adapter issues
   └── Final quality gate

4. PACKAGE PHASE (single agent, Sonnet)
   ├── Generate documentation
   ├── Package as skill/agent definitions
   └── Publish
```

### Git Worktree Isolation

Each agent gets its own worktree so they can edit files in parallel without conflicts:

```bash
# Agent-A works on CLI adapter
git worktree add ../hex-intf-cli feat/cli-adapter

# Agent-B works on Git adapter
git worktree add ../hex-intf-git feat/git-adapter

# After both complete, merge back
git merge feat/cli-adapter
git merge feat/git-adapter
```

### Ruflo Task Tracking

```
swarm_init(topology: "hierarchical", maxAgents: 8)
  ├── task_create("plan-workplan", assignee: "planner")
  ├── task_create("impl-cli-adapter", assignee: "coder-1")
  ├── task_create("impl-git-adapter", assignee: "coder-2")
  ├── task_create("impl-llm-adapter", assignee: "coder-3")
  ├── task_create("test-integration", assignee: "tester")
  └── task_create("package-skills", assignee: "packager")
```

---

## Multi-Language Support

### Language Matrix

| Capability | TypeScript | Go | Rust |
|-----------|-----------|-----|------|
| **Tree-Sitter** | `tree-sitter-typescript` | `tree-sitter-go` | `tree-sitter-rust` |
| **Linter** | ESLint + tsc strict | golangci-lint | clippy |
| **Build** | tsc / esbuild | go build | cargo build |
| **Test** | vitest / jest | go test | cargo test |
| **Compile Speed** | ~1s (esbuild) | ~2s | ~5-30s |
| **Type Safety** | Strong (strict mode) | Strong | Very Strong |

### Language-Specific Port Implementations

Each language gets its own adapter implementations. The port interfaces remain consistent across languages via code generation:

```
ports/IBuildPort.ts  →  generates  →  ports/build_port.go
                                       ports/build_port.rs
```

---

## Quality Gates

Every code generation cycle passes through:

1. **Compile** — Does it build? (< 5s feedback)
2. **Lint** — Does it pass static analysis? (< 2s feedback)
3. **Unit Test** — Do isolated tests pass? (< 10s feedback)
4. **Integration Test** — Do adapters work together? (< 30s feedback)
5. **AST Diff** — Did the structure change as expected? (< 1s feedback)
6. **Token Budget** — Is the summary still within budget? (< 1s feedback)

---

## Packaging & Reuse

### As a Claude Code Skill

```yaml
# skills/hex-intf-scaffold.yaml
name: hex-intf-scaffold
description: Scaffold a new hexagonal architecture project with LLM-optimized structure
triggers:
  - "create hex project"
  - "scaffold hexagonal"
  - "new ports and adapters project"
```

### As Agent Definitions

```yaml
# agents/hex-coder.yaml
name: hex-coder
type: coder
description: Generates code within a single hexagonal adapter boundary
context:
  - Load L1 summaries of all ports
  - Load L2 summary of target adapter
  - Load L3 of specific file being edited
constraints:
  - Never import across adapter boundaries
  - Always implement port interface fully
  - Always include unit tests
```

### As npm Package

```bash
npx hex-intf init --lang typescript --name my-project
npx hex-intf generate --adapter secondary/database --from spec.md
npx hex-intf summarize --level L2 --output context.txt
```

---

## Comparison with Existing Tools

| Feature | SPECKIT | BMAD | **hex-intf** |
|---------|---------|------|-------------|
| PRD Generation | Yes | Yes | Via workplan use case |
| Context Management | Basic | Prompt injection | Tree-sitter AST summaries (10x compression) |
| Architecture | Unstructured | Template-based | Hexagonal (ports & adapters) |
| Multi-Language | JS only | Various | TS + Go + Rust (first-class) |
| Testability | Manual | Partial | TDD London School (100% port coverage) |
| Swarm Support | No | No | Ruflo + git worktrees |
| Token Efficiency | Low | Medium | High (L0-L3 progressive loading) |
| Reusable | Templates | Templates | Skills + Agent defs + npm package |

---

## ADR Index

- [ADR-001](../adrs/ADR-001-hexagonal-architecture.md) — Hexagonal Architecture as foundational pattern
- [ADR-002](../adrs/ADR-002-treesitter-summaries.md) — Tree-sitter for token-efficient LLM communication
- [ADR-003](../adrs/ADR-003-multi-language.md) — TypeScript, Go, Rust as supported languages
- [ADR-004](../adrs/ADR-004-swarm-worktrees.md) — Git worktrees for parallel agent isolation
- [ADR-005](../adrs/ADR-005-quality-gates.md) — Compile → Lint → Test feedback loop
- [ADR-006](../adrs/ADR-006-packaging.md) — Skills, agent definitions, and npm packaging

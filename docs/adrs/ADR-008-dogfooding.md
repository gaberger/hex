# ADR-008: Dogfooding — hex-intf Built with Hexagonal Architecture

## Status: Accepted
## Date: 2026-03-15

## Context

hex-intf is a framework that prescribes hexagonal architecture for AI-driven development projects. If the framework itself doesn't follow its own patterns, we lose:

1. **Credibility** — users won't trust patterns we don't use ourselves
2. **Validation** — we can't prove the patterns work at framework scale
3. **Self-hosting** — hex-intf agents should be able to modify hex-intf itself using hex-intf's own tooling (L2 summaries, quality gates, feedback loops)
4. **Consistency** — tree-sitter summaries of hex-intf's code should match the patterns documented in its architecture spec

## Decision

hex-intf is built using its own hexagonal architecture:

- **Domain Core** (`src/core/domain/`, `src/core/usecases/`): Pure business logic — feedback loops, quality scoring, task graphs, event handling. Zero external dependencies.
- **Port Interfaces** (`src/core/ports/`): Typed contracts for all boundaries — AST, LLM, Build, Git, Worktree, FileSystem, Notification. No implementation details.
- **Primary Adapters** (`src/adapters/primary/`): CLI, Agent API, HTTP — drive the use cases.
- **Secondary Adapters** (`src/adapters/secondary/`): TreeSitter, Git, LLM providers, terminal notifier, webhook — implement output ports.

### Compliance Rules

1. `src/core/domain/` may only import from `src/core/ports/`
2. `src/core/usecases/` may only import from `src/core/domain/` and `src/core/ports/`
3. `src/adapters/` may only import from `src/core/ports/` (never from domain or other adapters)
4. No adapter may import another adapter — cross-cutting concerns go through the domain
5. All external libraries (tree-sitter, simple-git, etc.) are wrapped in adapters
6. The notification system is driven by DomainEvents, not by adapters directly

### Self-Hosting Test

hex-intf passes the "self-hosting test" when:
- `hex-intf summarize --level L2` produces valid summaries of its own codebase
- `hex-intf generate --adapter secondary/treesitter` can be used to modify the tree-sitter adapter
- Quality gates run on hex-intf's own code during development
- Agents can work on hex-intf using hex-intf's worktree isolation

## Consequences

- **Positive**: Framework is validated by its own usage — bugs surface immediately
- **Positive**: L2 summaries of hex-intf serve as both documentation and test data
- **Positive**: Contributors use the same patterns they're building
- **Negative**: Bootstrap problem — early development can't use the framework's own tooling (solved by progressive bootstrapping)
- **Negative**: Stricter constraints slow initial development (acceptable tradeoff)

## Enforcement

- CI lint rule: `no-cross-boundary-imports` checks dependency direction
- PR template includes dogfooding checklist
- Tree-sitter summary of every changed file must be valid L2

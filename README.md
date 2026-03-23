<p align="center">
  <img src=".github/assets/banner.svg" alt="hex — Hexagonal Architecture Harness for AI-Driven Development" width="900"/>
</p>

<p align="center">
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-blue?style=flat-square" alt="MIT License"/></a>
  <a href="#multi-language-support"><img src="https://img.shields.io/badge/languages-TS%20%7C%20Go%20%7C%20Rust-informational?style=flat-square" alt="Languages"/></a>
  <a href="#multi-agent-swarm-coordination"><img src="https://img.shields.io/badge/swarm-HexFlo%20powered-blueviolet?style=flat-square" alt="Swarm"/></a>
</p>

<p align="center">
  <b>An AI-Assisted Integrated Development Environment (AAIDE) with opinionated hexagonal architecture enforcement.</b>
</p>

---

## What is hex?

**hex** is a framework and toolchain that enforces hexagonal architecture (Ports & Adapters) during AI-driven development. It is not an application you deploy — it gets installed into target projects.

hex provides:

- **Mechanical architecture enforcement** via static analysis (`hex analyze`)
- **Multi-agent swarm coordination** for parallel feature development across adapter boundaries
- **Provider-agnostic enforcement** — works with Claude, Ollama, vLLM, Qwen, Llama, or any MCP-compatible client
- **Token-efficient code context** via tree-sitter AST summaries (L0–L3)
- **A control plane dashboard** for managing development across projects and systems
- **Three-layer enforcement** — client hooks + MCP tool guards + server-side API middleware

## Quick Start

```bash
# Build hex from source
cargo build -p hex-cli --release

# Initialize hex in your project
./target/release/hex init

# Start the nexus daemon (dashboard at http://localhost:5555)
hex nexus start

# Check architecture health
hex analyze .

# List Architecture Decision Records
hex adr list

# Show enforcement mode
hex enforce mode
```

## System Architecture

hex is composed of five deployment units:

| Component | Purpose |
|:----------|:--------|
| **SpacetimeDB** | Coordination & state core — 19 WASM modules, real-time WebSocket sync |
| **hex-nexus** | Filesystem bridge daemon — REST API, dashboard, architecture analysis |
| **hex-agent** | Architecture enforcement runtime — skills, hooks, ADRs, workplans |
| **hex-cli** | Rust CLI binary — canonical user entry point for all hex commands |
| **hex-dashboard** | Solid.js control plane — multi-project monitoring at `:5555` |

See [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) for detailed component documentation.

## CLI Commands

| Command | Description |
|:--------|:-----------|
| `hex init` | Initialize hex in a project directory |
| `hex analyze .` | Architecture health check (boundaries, dead code, ADR compliance) |
| `hex nexus start\|stop\|status` | Manage the hex-nexus daemon |
| `hex adr list\|search\|schema` | Architecture Decision Records |
| `hex swarm init <name>` | Initialize a HexFlo swarm |
| `hex task create\|list\|complete` | Swarm task management |
| `hex plan create\|list\|status` | Workplan management |
| `hex agent list\|id\|audit` | Agent management and tracking audit |
| `hex test unit\|all\|history\|trends` | Run tests and view trends |
| `hex enforce list\|sync\|mode\|prompt` | Enforcement rule management |
| `hex memory store\|get\|search` | Persistent key-value memory |
| `hex inbox list\|notify\|ack` | Agent notification inbox |
| `hex mcp` | Start MCP server (stdio transport) |
| `hex assets` | List embedded assets in the binary |
| `hex status` | Project status overview |

## Enforcement

hex enforces the **ADR → Workplan → Swarm → Agent** pipeline through three layers:

```
Layer 1: Client hooks     (Claude Code)   → pre-agent, pre-edit, pre-bash
Layer 2: MCP tool guards  (any provider)  → DefaultEnforcer before dispatch
Layer 3: Nexus API guards (server-side)   → axum middleware (403 on violation)
```

| Check | Mandatory Mode | Advisory Mode |
|:------|:--------------|:-------------|
| Background agent without `HEXFLO_TASK:` | Blocked | Warning |
| Mutating operation without active workplan | Blocked | Warning |
| Unregistered agent | Warning | Allowed |

For non-MCP models (Ollama, vLLM): `hex enforce prompt` generates system prompt instructions.

See [docs/ENFORCEMENT.md](docs/ENFORCEMENT.md) for the full enforcement architecture.

## Hexagonal Architecture Rules

These rules are checked by `hex analyze`:

1. `domain/` must only import from `domain/`
2. `ports/` may import from `domain/` but nothing else
3. `usecases/` may import from `domain/` and `ports/` only
4. `adapters/primary/` may import from `ports/` only
5. `adapters/secondary/` may import from `ports/` only
6. **Adapters must NEVER import other adapters**
7. `composition-root` is the ONLY file that imports from adapters

## ADR Numbering

New ADRs use timestamp-based IDs (`ADR-YYMMDDHHMM`, e.g., `ADR-2603221500`) to eliminate race conditions in concurrent multi-agent creation. Legacy sequential IDs (`ADR-001` through `ADR-066`) are preserved.

```bash
hex adr schema    # Show next available ID + template
hex adr list      # List all ADRs with status
```

## Development

```bash
# Build
cargo build -p hex-cli --release       # CLI binary
cargo build -p hex-nexus --release     # Nexus daemon

# Test
cargo test -p hex-cli                  # Rust CLI tests
cargo test -p hex-core                 # Core domain tests
bun test tests/unit/ tests/smoke/      # TypeScript tests

# Analyze
hex analyze .                          # Architecture health
hex enforce list                       # Enforcement rules
hex agent audit                        # Track untracked commits
```

## Project Structure

```
hex-cli/          # Rust CLI binary (canonical entry point)
hex-nexus/        # Filesystem bridge daemon + dashboard
hex-core/         # Shared domain types & port traits
hex-agent/        # Architecture enforcement runtime
hex-desktop/      # Desktop app (Tauri wrapper)
hex-parser/       # Code parsing utilities
spacetime-modules/ # 19 SpacetimeDB WASM modules
docs/adrs/        # Architecture Decision Records
docs/workplans/   # Feature workplans
```

## License

MIT

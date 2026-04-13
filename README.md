<p align="center">
  <img src=".github/assets/banner.svg" alt="hex — AI Operating System" width="900">
</p>

<p align="center">
  <a href="https://www.rust-lang.org/"><img src="https://img.shields.io/badge/Rust-1.75+-dea584?style=flat-square&logo=rust&logoColor=white" alt="Rust"></a>
  <a href="https://spacetimedb.com/"><img src="https://img.shields.io/badge/SpacetimeDB-WASM-58a6ff?style=flat-square" alt="SpacetimeDB"></a>
  <a href="https://github.com/gaberger/hex/blob/main/LICENSE"><img src="https://img.shields.io/badge/License-MIT-3fb950?style=flat-square" alt="License"></a>
  <a href="docs/adrs/"><img src="https://img.shields.io/badge/ADRs-154_(144_Accepted)-bc8cff?style=flat-square" alt="ADRs"></a>
  <a href="#architecture-enforcement-that-agents-cant-bypass"><img src="https://img.shields.io/badge/Architecture-A%2B_100%2F100-3fb950?style=flat-square" alt="Architecture A+"></a>
</p>

<p align="center">
  <strong>The operating system for AI agents — local-first, self-improving, zero cloud required.</strong><br>
  <em>Run coding agents on your own hardware. Enforce architecture. Coordinate swarms. Get smarter every run.</em>
</p>

---

## The Problem

AI coding agents are powerful — but they're expensive, uncontrolled, and cloud-dependent. Every task pays the same price regardless of complexity. A typo fix costs as much as a feature implementation. And when you scale to multiple agents, you get conflicting edits, architecture violations, and no coordination.

**What if 70% of your agent tasks could run on a $0/month local model — with the same quality as frontier?** hex classifies tasks by complexity, routes simple work to fast local models, and only escalates to cloud when the task genuinely needs it. The system learns from every dispatch and gets better over time.

---

## What You Get

- **[Architecture Enforcement](docs/ARCHITECTURE.md#architecture-enforcement-that-agents-cant-bypass)** — Tree-sitter boundary checking at the kernel level. Agents physically cannot violate hex rules. Grade: A+ (100/100).
- **[Tiered Inference](docs/INFERENCE.md)** — T1-T3 routing: 4B for typos (68 tok/s), 32B for codegen (11 tok/s), frontier only when needed. RL Q-learning self-optimizes over time.
- **[Code-First Execution](docs/INFERENCE.md#code-first-execution)** — Templates, AST transforms, and scripts before inference. 14 of 20 workplan tasks need zero tokens.
- **[Swarm Coordination](docs/ARCHITECTURE.md#swarm-coordination-without-merge-conflicts)** — HexFlo: native Rust, <1ms latency, isolated git worktrees, heartbeat recovery.
- **[Developer Experience](docs/DEVELOPER-EXPERIENCE.md)** — 4 progressive layers (Pulse/Brief/Console/Override), trust delegation, taste graph.
- **[Standalone Mode](docs/GETTING-STARTED.md#running-hex-standalone--zero-cloud-dependencies)** — Ollama + any GGUF model. No API keys, no cloud accounts, no billing.
- **[Agent Security](docs/ARCHITECTURE.md#capability-based-agent-security)** — HMAC-SHA256 capability tokens, least-privilege scoping, zero secrets in storage.

---

## Quick Start

```bash
cargo build -p hex-cli --release && cargo build -p hex-nexus --release
hex nexus start        # Requires SpacetimeDB running
hex status
open http://localhost:5555   # Live dashboard
```

See [Getting Started](docs/GETTING-STARTED.md) for full installation, essential commands, and standalone setup.

---

## System Architecture

```
hex-cli/               Rust CLI — shell + MCP server (canonical entry point)
hex-nexus/             Daemon — REST API, dashboard, filesystem bridge
hex-core/              Domain types + 10 port traits (zero external deps)
hex-agent/             Agent runtime — skills, hooks, architecture enforcement
hex-parser/            Code parsing utilities (tree-sitter)
spacetime-modules/     7 WASM modules (SpacetimeDB microkernel)
```

See [Architecture](docs/ARCHITECTURE.md) for crate details, agent roles, enforcement rules, and the SpacetimeDB microkernel.

---

## Documentation

| Doc | Description |
|:----|:------------|
| [Architecture](docs/ARCHITECTURE.md) | System components, enforcement rules, agent roles, swarm coordination, security |
| [Getting Started](docs/GETTING-STARTED.md) | Installation, essential commands, standalone mode, remote agents |
| [Inference](docs/INFERENCE.md) | Tiered routing, RL self-improvement, code-first execution, GBNF grammars, benchmarking |
| [Comparison](docs/COMPARISON.md) | hex vs BAML, SpecKit, HUD, LangChain, CrewAI, AutoGen, Claude Agent SDK |
| [Developer Experience](docs/DEVELOPER-EXPERIENCE.md) | Pulse/Brief/Console/Override layers, trust delegation, workplan dispatch |
| [Architecture Decision Records](docs/adrs/) | 154 decisions (144 accepted) — the "why" behind every design choice |
| [Development Guides](docs/guides/) | Workflow walkthrough, OpenRouter setup, feature UX integration |
| [Behavioral Specs](docs/specs/) | Feature specifications written before code |
| [Workplans](docs/workplans/) | Structured task decomposition driving HexFlo swarm execution |

---

## Who Is This For?

**Teams running AI coding agents at scale — especially those who want to own their inference.** If you're using Claude Code, Copilot, Cursor, or any LLM-powered coding tool and you've hit these problems:

- Agents violate architecture boundaries and you find out in code review
- Multi-agent runs produce merge conflicts or race conditions
- Every agent has full access to everything — no scoping, no least privilege
- You're paying frontier API prices for tasks a local model could handle
- You want to run agents on airgapped networks, on-prem hardware, or without cloud accounts

hex installs into your project as the operating system layer between your agents and your codebase. It's model-agnostic, provider-agnostic, and runs on local models by default.

---

## Credits & References

### Foundational Work

hex builds on the **Hexagonal Architecture** pattern (Ports & Adapters), originally conceived by [Alistair Cockburn](https://alistair.cockburn.us/hexagonal-architecture/) in 2005:

> *"Allow an application to equally be driven by users, programs, automated test or batch scripts, and to be developed and tested in isolation from its eventual run-time devices and databases."*

- **[Hexagonal Architecture](https://alistair.cockburn.us/hexagonal-architecture/)** — Alistair Cockburn
- **[Growing Object-Oriented Software, Guided by Tests](http://www.growing-object-oriented-software.com/)** — Steve Freeman & Nat Pryce (London-school TDD)
- **[Clean Architecture](https://blog.cleancoder.com/uncle-bob/2012/08/13/the-clean-architecture.html)** — Robert C. Martin

### Key Technologies

- **[tree-sitter](https://tree-sitter.github.io/)** — Max Brunsfeld et al. (AST-based architecture enforcement)
- **[SpacetimeDB](https://spacetimedb.com/)** — real-time database with WASM module execution
- **[claude-flow](https://github.com/ruvnet/claude-flow)** — Reuven Cohen (@ruvnet), multi-agent swarm coordination (predecessor to HexFlo)

### Authors

| Contributor | Role |
|:------------|:-----|
| **Gary** ([@gaberger](https://github.com/gaberger)) | Creator, architect, primary developer |
| **Claude** (Anthropic) | AI pair programmer — code generation, testing, documentation |

---

## License

[MIT](LICENSE)

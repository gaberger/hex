# hex — AI-Assisted Integrated Development Environment

hex is an **AAIDE** — an opinionated development framework built around **hexagonal architecture** (Ports & Adapters). It is not an application you deploy; it is the framework + CLI toolchain that gets installed into target projects to enforce architecture and coordinate AI-driven development.

## System Components

| Component | Role |
|-----------|------|
| **hex-cli** | CLI binary — all `hex` commands, MCP server |
| **hex-nexus** | Filesystem bridge daemon + dashboard (port 5555) |
| **hex-agent** | Architecture enforcement runtime for AI agents |
| **hex-core** | Shared domain types and port traits (zero external deps) |
| **spacetime-modules** | 7 WASM modules for coordination, inference, swarms (ADR-2604050900) |

SpacetimeDB must be running. All clients connect via WebSocket for real-time state sync.

## Quick Start

```bash
# Build
cargo build -p hex-cli
cargo build -p hex-nexus --release

# Start the nexus daemon
hex nexus start

# Check status
hex status
hex nexus status
```

## Inference Routing

hex includes quantization-aware inference routing (ADR-2603271000). Inference requests are automatically routed to the best available provider based on task complexity and provider quality scores.

### How it works

1. **Complexity scoring** — every inference request is scored on prompt length, cross-file keywords, security signals, and architectural keywords
2. **Minimum tier selection** — complexity maps to a minimum quantization tier (Low→Q2, Medium→Q4, High→Q8, Critical→Cloud)
3. **Quality-ranked selection** — providers meeting the minimum tier are ranked by calibrated `quality_score` (0.0–1.0)
4. **Escalation on failure** — if a provider fails, the router retries at the next tier (Q4→Q8→Fp16→Cloud)

### Quantization tiers

| Tier | Bits | Use cases |
|------|------|-----------|
| Q2 | 2-bit | Scaffolding, docstrings, formatting |
| Q4 | 4-bit | General coding, test generation |
| Q8 | 8-bit | Complex reasoning, security review |
| Fp16 | 16-bit | Cross-file planning |
| Cloud | — | Frontier APIs (Anthropic, OpenAI, OpenRouter) |

### Provider registry

```bash
# Discover and register OpenRouter models
hex inference discover --provider openrouter

# List all registered providers with quality scores
hex inference list

# Calibrate a provider (runs real inference, writes quality_score)
hex inference test <provider-id>

# Batch-calibrate all uncalibrated providers
# POST http://localhost:5555/api/neural-lab/experiments/quant-calibration

# Add a local Ollama provider (quantization auto-detected from model tag)
hex inference add ollama http://localhost:11434 llama3.2:3b-q4_k_m
```

Quality scores are stored in SpacetimeDB and used immediately for routing. Once calibrated, the model router uses `SelectionSource::RegistryRanked` — no hardcoded model strings.

### Agent quantization policy

Agent YAMLs declare quantization requirements:

```yaml
# hex-coder.yml — routine coding
inference:
  quantization:
    default: q4
    minimum: q2
    on_complexity_high: q8
    on_failure: cloud

# planner.yml — architectural planning
inference:
  quantization:
    default: q8
    minimum: q4
    on_complexity_high: cloud
    on_failure: cloud
```

## Development Pipeline

Features follow a specs-first lifecycle:

```
ADR → Specs → Workplan → Code (TDD, parallel worktrees) → Validate → Merge
```

```bash
hex adr list              # List all ADRs
hex adr status <id>       # Show ADR detail
hex swarm init <name>     # Start a swarm
hex task list             # List swarm tasks
hex memory store <k> <v>  # Persist key-value
hex inbox list            # Agent notification inbox
```

## Architecture Rules

Enforced by `hex analyze .`:

- `domain/` — pure business logic, no external imports
- `ports/` — typed interfaces, imports from domain only
- `adapters/secondary/` — driven adapters, import from ports only
- `adapters/primary/` — driving adapters, import from ports only
- **Adapters must never import other adapters**
- `composition-root` is the only file that imports from adapters

```bash
hex analyze .             # Architecture health check
cargo test -p hex-nexus --test quant_routing   # Routing tests (19/19)
cargo check -p hex-cli -p hex-nexus -p hex-core
```

## Key ADRs

| ADR | Description | Status |
|-----|-------------|--------|
| ADR-2603271000 | Quantization-Aware Inference Routing | Implemented |
| ADR-2603261000 | Secure Inference Provider Registry | Implemented |
| ADR-2603301200 | Architecture Context Injection (ACI) | Active |
| ADR-2603300100 | hex-agent SpacetimeDB WebSocket Client | Active |
| ADR-2603301600 | Batch Command Execution Context Indexing | Active |
| ADR-027 | HexFlo Native Coordination | Implemented |

## Architecture Decisions

<!-- hex:adr-summary — auto-updated by hex -->
| ADR | Title | Status |
|-----|-------|--------|
| 001 | Hexagonal Architecture as Foundational Pattern | Accepted |
| 002 | Tree-Sitter for Token-Efficient LLM Communication | Accepted |
| 003 | Multi-Language Support — TypeScript, Go, Rust | Accepted |
| 004 | Git Worktrees for Parallel Agent Isolation | Accepted |
| 005 | Compile-Lint-Test Feedback Loop with Quality Gates | Accepted |
| 006 | Skills, Agent Definitions, and npm Packaging | Accepted |
| 007 | Multi-Channel Notification System | Accepted |
| 008 | Dogfooding — hex Built with Hexagonal Architecture | Accepted |
| 009 | Ruflo (claude-flow) as Required Dependency | Superseded by ADR-027 (HexFlo) |
| 010 | TypeScript-to-Rust Migration Cost and Risk Analysis | Accepted |
| 011 | Coordination and Multi-Instance Locking | Accepted |
| 012 | ADR Lifecycle Tracking | Accepted |
| 013 | Secrets Management | Accepted |
| 014 | Ban mock.module() — Use Dependency Injection for Test Isolation | Accepted |
| 015 | SQLite Persistence for Hub Swarm State | Accepted |
| 016 | Hub Binary Version Verification | Superseded by ADR-032 |
| 017 | Unlink Binary Before Copy to Avoid macOS Inode-Based SIGKILL Cache | Accepted |
| 018 | Multi-Language Build Enforcement (Go + Rust) | Accepted |
| 019 | CLI–MCP Parity — Every Command Must Have an MCP Equivalent | Accepted |
| 020 | Feature Development UX Improvement | Accepted |
| 021 | Hex Initialization Memory Exhaustion in Existing Large Projects | Accepted |
| 022 | Wire Coordination into Use Cases (Last-Mile Fix) | Accepted |
| 023 | Dashboard Session Cleanup and State Synchronization | Accepted |
| 024 | Hex-Hub Autonomous Nexus Architecture | Accepted |
| 025 | SpacetimeDB as Distributed State Backend | Accepted |
| 026 | Secure Secret Distribution via SpacetimeDB Coordination | Accepted |
| 027 | HexFlo — Replace Ruflo with Native Swarm Coordination | Accepted |
| 028 | API Optimization Layer | Accepted |
| 029 | Haiku Preflight Checks & Automatic Context Compaction | Accepted |
| 030 | Multi-Provider Inference Broker | Accepted |
| 031 | RL-Driven Model Selection & Token Budget Management | Accepted (documenting existing implementation) |
| 032 | Deprecate hex-hub — Consolidate into hex-nexus and hex-agent | Accepted |
| 033 | MCP Client Support for hex-agent | Accepted |
| 034 | Migrate Hex Analyzer from TypeScript to Rust | Accepted |
| 035 | Hex Architecture V2 — Rust-First, SpacetimeDB-Native, Pluggable Inference | Accepted |
| 036 | hex-chat Session Architecture | Deprecated — hex-chat removed (2026-03-22) |
| 037 | Agent Lifecycle — Local Default + Remote Connect | Accepted |
| 038 | Vite for Development, Axum for Production | Accepted |
| 039 | Nexus Agent Control Plane — OpenCode-Inspired Multi-Project Interface | Accepted |
| 040 | Remote Agent Transport — WebSocket over SSH with SpacetimeDB Coordination | Accepted |
| 041 | ADR Review Agent — Architectural Consistency Guardian | Accepted |
| 042 | SpacetimeDB Skill Lifecycle — Ingest, Select, Serialize | Accepted |
| 043 | Project Manifest + Auto-Registration via SpacetimeDB | Accepted |
| 044 | Nexus Git Integration — Project-Scoped Git Intelligence | Accepted |
| 045 | Project-Scoped ADRs, Config Templates, and Embedded Chat | Accepted |
| 046 | SpacetimeDB Single Authority for State Mutations | Accepted |
| 047 | Internal Documentation System | Accepted |
| 048 | Claude Code Session Agent Registration | Accepted |
| 049 | Embedded Settings Template — Single Source of Truth | Accepted |
| 050 | Hook-Enforced Agent Lifecycle Pipeline | Accepted |
| 051 | SpacetimeDB as Single Source of State | Accepted |
| 052 | AIIDE — Hex Nexus as AI Integrated Development Environment | Accepted |
| 053 | Framework Configuration Sync to SpacetimeDB | Accepted |
| 054 | ADR Compliance Enforcement — Preventing Architectural Drift | Accepted |
| 055 | README-Driven Project Specification | Accepted |
| 056 | Frontend Hexagonal Architecture — Preventing UI Species Drift | Accepted |
| 057 | Unified Test Harness & Linting Pipeline | Accepted |
| 058 | Test Session Persistence and Outcome Tracking | Accepted |
| 059 | Canonical Project Identity Contract | Accepted |
| 060 | Agent Notification Inbox | Accepted |
| 061 | Workplan Lifecycle Management — Creation, Tracking, and Reporting | Accepted |
| 062 | Unified Agent Identity — Single Registry, Reliable Resolution | Superseded |
| 063 | Deprecate SQLite, Migrate HexFlo to SpacetimeDB | Accepted |
| 064 | Rust Compilation Performance | Accepted |
| 065 | Registration Lifecycle Gaps — Project and Agent Identity | Accepted |
| 066 | Dashboard Visibility Overhaul | Accepted |
| 067 | Hex Installation & Pipeline Validation | Accepted |
| 2603221500 | Timestamp-Based ADR Numbering (YYMMDDHHMM) | Accepted |
| 2603221522 | Embedded Asset Bundle — rust-embed for CLI Templates and Schemas | Accepted |
| 2603221939 | Mandatory Swarm Tracking for Background Agents | Accepted |
| 2603221959 | Provider-Agnostic Enforcement via MCP Tool Guards | Accepted |
| 2603222035 | Dependency Vulnerability Remediation | Proposed |
| 2603222050 | Remove Legacy TypeScript CLI and Adapters | Proposed |
| 2603222136 | README Restructure — Accurate, Modular Documentation | Accepted |
| 2603222229 | CLI / MCP / Dashboard Parity Investigation | Proposed |
| 2603231000 | Dashboard Reactive Context Fix — Eliminate Module-Level Computations | Accepted |
| 2603231309 | Map All hex CLI Commands Into Dashboard UI | Accepted |
| 2603231400 | SpacetimeDB Operational Resilience | Accepted |
| 2603231500 | SpacetimeDB Per-Module Databases | Accepted |
| 2603231600 | OpenRouter Inference Integration | Accepted |
| 2603231700 | Worktree Enforcement in Agent Hooks | Accepted |
| 2603231800 | hex Context Injection into opencode | Accepted |
| 2603231900 | Fix `hex test all` False Skips | Accepted |
| 2603232000 | Swarm-Gate Enforcement at Pre-Agent Hook | Accepted |
| 2603232005 | Self-Sufficient hex-agent with TUI | Accepted |
| 2603232216 | hex dev Pipeline Validation Report | Accepted |
| 2603232220 | Developer Audit Report — Full Pipeline Traceability | Accepted |
| 2603232230 | Tool Call Tracking in SpacetimeDB | Proposed |
| 2603232340 | Validate Loop — Test, Analyze, Refactor Until Grade A | Proposed |
| 2603240045 | Free Model Performance Tracking in SpacetimeDB | Proposed |
| 2603240104 | Swarm Agent Personalities — Specialized Roles with Context-Aware Prompting | Accepted |
| 2603240130 | Declarative Swarm Agent Behavior from YAML Definitions | Accepted |
| 2603241126 | TUI CLI Surrogate + Pipeline Traceability | Accepted |
| 2603241226 | Structured CLI Table Output via `tabled` | Accepted |
| 2603241230 | Neural Network Encoding in SpacetimeDB WASM | Accepted |
| 2603241230 | Persistent Agent Coordination via SpacetimeDB | Accepted |
| 2603241430 | TUI Non-Blocking Phase Execution | Superseded by ADR-2603241500 |
| 2603241500 | TUI Async Channel Architecture | Accepted |
| 2603241800 | Swarm Lifecycle Management (Complete / Fail / Cleanup) | Proposed |
| 2603241900 | Agent-Swarm Ownership Hierarchy with Conflict Detection | Accepted |
| 2603242100 | Comprehensive hex-cli Integration Testing | Proposed |
| 2603250838 | CLI / MCP Shared Implementation — One Function, Two Skins | Accepted |
| 2603250900 | Reviewer RL Integration and Structured-Output Reliability | Accepted |
| 2603261000 | Secure Inference Provider Registry and Encrypted Secrets Vault | Accepted |
| 2603271000 | Quantization-Aware Inference Routing | Accepted |
| 2603281000 | Context Pipeline Compression | Accepted |
| 2603282000 | hex-agent as Claude Code-Independent Runtime in Docker AI Sandbox | Accepted |
| 2603283000 | Rust Workspace Boundary Analysis in hex analyze | Accepted |
| 2603291900 | Docker Worker First-Class Execution | Proposed |
| 2603300100 | hex-agent as First-Class SpacetimeDB WebSocket Client | Accepted |
| 2603301200 | Architecture Context Injection for Inference | Proposed |
| 2603301600 | Batch Command Execution with Context Indexing | Proposed |
| 2603311000 | Workflow Reliability Hardening | Accepted |
| 2603311711 | Static Site Generator for hex Documentation | proposed |
| 2603311730 | Integrate claude-code Capabilities into hex-agent | Accepted |
| 2603311900 | Pipeline Phase Pre-condition Gates | Accepted |
| 2603312000 | hex docs — Static Site Generator for the hex Manual | Proposed |
| 2603312100 | Context Engineering for hex-agent | Proposed |
| 2603312210 | Claude Code Bypass Mode for hex-agent | Proposed |
| 2603312300 | Workplan Live Execution Overlay in `hex plan list` | Proposed |
| 2603312305 | Inference Provider Health Checks and Pruning | proposed |
| 2603312332 | Inference Provider Quality Gates and Pruning | proposed |
| 2603312337 | Real-time Development Session Tracking via Push API | proposed |
<!-- /hex:adr-summary -->

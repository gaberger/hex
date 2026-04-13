# ADR-2604131500: AIOS Developer Experience — Ambient Autonomy with On-Demand Depth

**Status:** Accepted
**Date:** 2026-04-13
**Drivers:** hex has no coherent developer-facing UX for autonomous operation. The dashboard is a passive status display. The CLI is command-driven. Neither matches hex's identity as an autonomous AI operating system that builds software while developers steer from above. Developers currently must sit at a Claude Code prompt to use hex — this ADR defines the experience that makes hex a standalone AIOS.
**Supersedes:** ADR-066 (Dashboard Overhaul — this ADR subsumes dashboard redesign into a broader experience architecture)

## Context

### The Problem

hex can spawn agents, coordinate swarms, route inference, and enforce hexagonal architecture. But it has no coherent answer to: **"How does a developer actually use hex?"**

Today, a developer using hex must:
1. Open a Claude Code session
2. Type commands or invoke skills
3. Watch agent output scroll by
4. Manually check on progress
5. Intervene by typing more commands

This is a **prompt-driven, synchronous, single-session** model. It assumes the developer is the executor, with AI as assistant. But hex's architecture promises something fundamentally different: an **autonomous system that builds software while the developer provides direction**.

The gap between what hex can do and how developers interact with it is the single biggest barrier to hex achieving its vision.

### What Exists Today

| Surface | What it does | Limitation |
|---|---|---|
| Dashboard (localhost:5555) | Shows swarm status, task DAGs, agent fleet | Passive display — no decision-making, no steering |
| CLI (`hex` commands) | 37 subcommands for specific operations | Command-driven — developer must know what to type |
| Statusline | Shows project/branch/agent/swarm state | Information-only — no interaction |
| Inbox (ADR-060) | Agent-to-agent notifications | Agent-facing — no developer-facing decision flow |
| Claude Code integration | Spawns agents via Agent tool | Requires Claude Code session, not standalone |

### Forces

1. **hex should work while the developer is away.** The current model requires active developer presence.
2. **hex manages multiple projects simultaneously.** The current model is single-project, single-session.
3. **Developers don't want to watch dashboards.** They want to ship, not monitor.
4. **Autonomous systems need trust controls.** Without explicit trust levels, developers either over-supervise (defeating the purpose) or under-supervise (risking bad output).
5. **hex's value compounds across projects.** But today, each project starts from zero context about the developer's preferences.
6. **The agent runtime must be decoupled.** hex's UX should work regardless of whether the backend uses Claude Code, Ollama, or any other inference provider.

### Alternatives Considered

**Mission Control (rejected as primary, adopted for investigation layer):** A NASA-style dashboard with five consoles (Architecture, Swarm, Build, Inference, Timeline), structured Go/No-Go gates at every phase, and typed directives. Rejected because it creates an attention tax that scales linearly with project count, puts the developer back as a serial bottleneck, and rewards dashboard-watching over actual productivity. However, its investigation tools (Architecture Pressure Map, Token Market, Counterfactual Branching) are adopted as on-demand deep-dive capabilities.

**Pure Ambient / Living System (rejected as sole model, adopted as foundation):** A system where the developer never checks a dashboard and hex surfaces decisions contextually via Slack/email/mobile. Rejected as sole model because it provides no investigation depth when things go wrong and no auditability for professional software development. However, its core insight — that 90% of interactions should be ambient and the system should learn the developer's taste over time — is adopted as the default interaction layer.

### Research Basis

This ADR is the product of an adversarial research exercise with two competing design teams and an independent judge. Full proposals:
- `docs/analysis/team-alpha-mission-control-proposal.md`
- `docs/design/team-beta-living-system-proposal.md`
- `docs/design/judge-verdict-aios-experience.md`
- `docs/design/aios-developer-experience.md` (detailed interface specification)

## Decision

### 1. Four Interaction Layers: Ambient by Default, Deep on Demand

hex shall provide four distinct interaction layers. The system defaults to Layer 1 and escalates only when required. The developer moves deeper only when they choose to.

**Layer 1 — The Pulse (Always On)**

A single-line terminal statusline showing per-project state. Three symbols: `●` active, `◐` decisions pending, `◉` blocked, `○` idle. Extends the existing `hex-statusline.cjs`. Also available as macOS menu bar indicator via hex-desktop (Tauri).

The Pulse answers one question: "Do I need to do anything right now?"

**Layer 2 — The Briefing (`hex brief`)**

A narrative summary of what happened, what's happening, and what needs the developer's input. Pull-based (developer asks), not push-based (no Slack spam). Each pending decision includes the default choice, hex's reasoning, and a deadline after which the default auto-applies.

The Briefing is a newspaper, not a dashboard.

**Layer 3 — The Console (`hex console`)**

A context-aware investigation surface in the web dashboard. Shows relevant panels (Architecture, Swarm, Build, Inference, Timeline) based on what triggered the investigation. Used 1-2 times per week when something goes wrong or the developer is curious.

The Console is a microscope, not a cockpit.

**Layer 4 — The Override (`hex override`, `hex pause`, `hex steer`)**

Direct intervention commands for the 1% case where the developer needs to countermand the system. Typed CLI commands, not natural language.

The Override is an emergency brake, not a steering wheel.

### 2. Five New CLI Commands as Primary Interface

The developer's daily interaction with hex is through five CLI commands:

| Command | Purpose | Frequency |
|---|---|---|
| `hex brief` | Read narrative status + pending decisions | 1-3× daily |
| `hex decide <project> <id> <action>` | Approve/reject/override a pending decision | 2-5× daily |
| `hex steer <project> "<directive>"` | Natural language priority/approach change | 1-2× daily |
| `hex trust show/elevate/reduce` | Adjust autonomy levels per scope | 1-2× weekly |
| `hex new` | Structured project intake | As needed |

Target: **5-15 minutes of developer interaction per day** for 3-5 active projects.

### 3. Asynchronous Decisions with Defaults and Deadlines

hex shall never block waiting for developer input. Every decision point shall include:
- The default choice hex will take
- hex's reasoning (referencing Taste Graph, architectural rules, or past decisions)
- A deadline (configurable, default 2 hours) after which the default auto-applies
- One-line CLI commands to approve, reject, override, or request explanation

Auto-resolved decisions are logged in the briefing buffer and visible in the timeline.

### 4. Delegation Trust Model

A formal, inspectable trust model governs what hex can do without human approval.

**Trust levels:** `observe` (propose, wait indefinitely) → `suggest` (propose with default, auto-apply after deadline) → `act` (execute, notify in briefing) → `silent` (execute, log only)

**Trust scopes:** Hierarchical, matching hexagonal architecture: `project/domain/`, `project/ports/`, `project/adapters/primary/`, `project/adapters/secondary/<name>/`, `project/dependencies/`, `project/deployment/`

**Trust decay:** When a regression is attributable to an agent action (via git blame + test failure correlation), trust for that scope drops one level. The developer must explicitly re-elevate. hex never self-elevates.

**Trust bootstrapping:** New projects inherit trust from cross-project average minus one level (safety margin). First-ever project starts all scopes at `suggest`.

**Trust floor:** Destructive operations (file deletion, dependency removal, deployment, force-push) never auto-elevate past `act`.

### 5. Taste Graph — Cross-Project Preference Learning

hex shall maintain a persistent preference model that learns the developer's architectural and stylistic preferences and propagates them across projects.

**Three scopes:** Universal (all projects) → Language-specific (all Rust projects) → Domain-specific (all API projects)

**Learning mechanism:** Every developer edit to agent-generated code is recorded as a negative reward for the original and a positive reward for the developer's version. Preferences are extracted from the diff.

**Inspectable:** `hex taste list` shows all learned preferences with confidence scores. `hex taste set/forget/pin` for manual control. The system learns implicitly but exposes its learning explicitly.

### 6. Runtime-Agnostic Architecture

The AIOS experience layer (Pulse, Briefing, Trust, Steering, Decisions) shall be completely decoupled from the agent execution runtime via `IAgentRuntimePort`.

**Phase 1:** Ship UX with `ClaudeCodeRuntimeAdapter` (existing subprocess-based adapter) and/or `OpenCodeRuntimeAdapter` as backend.

**Phase 2:** Build `HexNativeRuntimeAdapter` — hex's own tool dispatcher that chains inference → tool call parsing → file edit/bash/read → result feedback → repeat.

**Phase 3:** hex runs fully standalone. Claude Code / opencode become optional backends, not requirements.

The port interface is: `execute_task(prompt, constraints) → TaskResult`. All orchestration, trust, taste, and UX code is runtime-agnostic.

### 7. Steering — Intent Without Implementation

`hex steer` accepts natural language directives that express intent, not tasks:
- "Finish the Slack adapter first, I need to demo Thursday"
- "Use SSE instead of WebSocket for real-time updates"
- "Optimize for simplicity over performance"

hex-nexus classifies the directive (priority change, approach change, constraint addition) and maps it to workplan modifications, agent prompt updates, and task reordering. The developer never needs to know agent IDs, task IDs, or workplan internals.

## Consequences

**Positive:**
- Developer interaction time drops from "active session" to 5-15 min/day
- hex works autonomously while the developer is away, at lunch, in meetings
- Multiple projects manageable simultaneously without context switching
- Trust model makes progressive autonomy concrete and auditable
- Taste Graph eliminates cold-start problem for new projects
- Runtime decoupling enables hex to become truly standalone

**Negative:**
- Significant new surface area: 5 CLI commands, 6 SpacetimeDB tables, 8+ REST endpoints, 6 dashboard components
- Trust calibration is an unsolved UX problem — too sensitive creates noise, too permissive enables drift
- Taste Graph may not generalize across dissimilar projects
- Narrative briefing generation requires either templates (brittle) or inference (expensive)

**Mitigations:**
- Phase 1 ships only Pulse + Brief + Decide + Trust (basic) — minimal surface area
- Trust starts conservative (all `suggest`) and the developer controls escalation
- Taste Graph v1 is manual-set + observation, not autonomous propagation
- Briefing uses templates first, with inference-generated narratives as Phase 3

## Implementation

| Phase | Description | Timeline | Status |
|-------|------------|----------|--------|
| P1 | Pulse + Brief + Decide + Trust (basic) — the "walk away" moment | 4-6 weeks | Pending |
| P2 | Token Market + Architecture Pressure + Taste v1 + Trust Decay | 8-12 weeks | Pending |
| P3 | Taste propagation + Immune System + Counterfactual + Codebase Instincts + Fleet | 16-24 weeks | Pending |
| P4 | HexNativeRuntimeAdapter — standalone agent execution | 8-12 weeks (parallel with P2-P3) | Pending |

## References

- `docs/design/aios-developer-experience.md` — Detailed interface specification with day-in-the-life scenarios
- `docs/design/judge-verdict-aios-experience.md` — Judge's verdict from adversarial research
- `docs/analysis/team-alpha-mission-control-proposal.md` — Mission Control proposal
- `docs/design/team-beta-living-system-proposal.md` — Living System proposal
- ADR-060 — Agent Notification Inbox (foundation for decision surfacing)
- ADR-066 — Dashboard Overhaul (subsumed by this ADR)
- ADR-2604112000 — Standalone Dispatch (runtime decoupling foundation)
- ADR-2604121630 — Nexus Coordinated Remote Execution

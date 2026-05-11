# hex Glossary

> **Canonical terminology for the hex AIOS.** This file is the single source of truth — all docs, code comments, ADRs, agent YAMLs, skills, and READMEs MUST use these terms. Stale / banned terms are listed in the **NOT** column so freshness checking (`hex docs check`, Phase 4 of ADR-047) can flag drift.
>
> **Authority:** ADR-047 (Internal Documentation System).
> **Last reviewed:** 2026-05-04.

## Core Terms

| Term | Definition | NOT |
|------|-----------|-----|
| **hex** | AI-Assisted Integrated Development Environment (AAIDE). The microkernel + adapter system that gets installed *into* target projects. | "harness", "framework" alone, "AIOS" alone (use "hex AIOS") |
| **AIOS** | AI Operating System — the hex runtime model (agents are users, developers are sysadmins). | generic "AI runtime" |
| **hex-nexus** | Filesystem-bridge daemon. Bridges the SpacetimeDB WASM sandbox ↔ local OS. Runs at `:5555`, serves the dashboard, exposes REST, syncs config → SpacetimeDB on startup (ADR-044). | "hex-hub", "hub", "orchestration nexus", "daemon" alone |
| **hex-agent** | Architecture-enforcement runtime. Deployed on any host running hex dev agents. Enforces hex rules via skills/hooks/ADRs/workplans. | "hex-adapter", "agent" alone (ambiguous w/ AI agent), hexagonal "adapter" concept |
| **hex-cli** | The `hex` binary — canonical user entry point. Delegates most work to `hex-nexus` REST. | "hex command", "CLI" alone |
| **hex-dashboard** | Solid.js + Tailwind control plane embedded in `hex-nexus`. Multi-project, fleet, arch-health, inference monitoring. | "dashboard" alone (ambiguous), "UI" alone |
| **hex-desktop** | Tauri wrapper around `hex-dashboard`. | "desktop app" alone |
| **hex-core** | Shared domain types & port traits crate (zero deps). | "core" alone |
| **hex-parser** | Code-parsing utilities crate (tree-sitter wrappers). | "parser" alone |

## Coordination & State

| Term | Definition | NOT |
|------|-----------|-----|
| **SpacetimeDB** | Coordination & state core. **Required** backbone service. Hosts 7 WASM modules. All clients connect via WebSocket. | "STDB" alone in user-facing docs (ok internally), "database" alone |
| **STDB** | Common abbreviation for SpacetimeDB. Acceptable in code/CLI output, not in user-facing prose. | substitute for "SpacetimeDB" in docs |
| **WASM module** | SpacetimeDB server-side logic unit (tables + reducers). Cannot access FS / spawn procs / make network calls — that's why hex-nexus exists. | "plugin", "extension", "service" alone |
| **reducer** | Transactional stored procedure inside a WASM module. Atomic, auditable, schema-validated. | "endpoint", "handler", "RPC" |
| **subscription** | SpacetimeDB push-stream of table-row changes. Powers live dashboard updates. | "websocket", "feed" alone |
| **HexFlo** | Native Rust swarm-coordination layer in `hex-nexus`. State persisted via the `hexflo-coordination` WASM module. ADR-027. | "ruflo" (deprecated predecessor), "claude-flow" |
| **hexflo-coordination** | WASM module: swarms, tasks, agents, memory, fleet. | "swarm module" alone |

## Architecture Layers

| Term | Definition | NOT |
|------|-----------|-----|
| **port** | Typed interface contract between architecture layers. Zero implementation. | "API", "service", "interface" alone |
| **adapter** | Implementation of a port for a specific technology (HTTP, FS, DB, etc.). | "plugin", "driver", "provider" |
| **primary adapter** | Driving adapter — accepts external input (CLI, HTTP, browser). Imports `ports/` only. | "inbound adapter" alone, "controller" |
| **secondary adapter** | Driven adapter — calls external systems (DB, API, FS). Imports `ports/` only. | "outbound adapter" alone, "repository" generically |
| **domain** | Pure business logic, zero external deps. May only import other `domain/` modules. | "model", "entity layer" |
| **usecase** | Application logic composing ports. May import `domain/` + `ports/` only. | "service", "handler" |
| **composition root** | The single file that wires adapters to ports (DI point). The ONLY file that imports from `adapters/`. | "config", "bootstrap", "container" |

## Inference

| Term | Definition | NOT |
|------|-----------|-----|
| **tier** | Routing class for inference: T1 (scaffold), T2 (codegen), T2.5 (reasoning), T3 (frontier). ADR-2026-04-12-0202. | "model size" alone |
| **strategy_hint** | Workplan-task field that selects a tier (`scaffold`/`transform`/`script` → T1, `codegen` → T2, `inference` → T2.5). | "model hint", "router hint" |
| **best-of-N** | T1/T2/T2.5 routing strategy: generate N candidates, select via compile gate (`cargo check` / `tsc --noEmit`). | "N-shot" |
| **inference-gateway** | WASM module that routes inference requests to providers. | "router" alone |
| **inference-bridge** | WASM module that hands routed requests off to `hex-nexus` for actual HTTP execution. | "bridge" alone |
| **standalone mode** | Runtime where `CLAUDE_SESSION_ID` is unset and hex-nexus uses `AgentManager` + `OllamaInferenceAdapter` directly. ADR-2026-04-11-2000. | "offline mode" |

## Workflow Artifacts

| Term | Definition | NOT |
|------|-----------|-----|
| **ADR** | Architecture Decision Record. Lives in `docs/adrs/`. Required for new ports/adapters/external deps/persistence/trust tiers. | "design doc", "RFC" alone |
| **workplan** | JSON task graph in `docs/workplans/`. Decomposes a feature into adapter-bounded tasks; one task = one adapter boundary = one git worktree. Max 8 parallel. | "plan" alone, "spec" |
| **behavioral spec** | User-facing acceptance criteria written BEFORE codegen. Lives in `docs/specs/`. Independent oracle for the validation-judge. | "test plan", "acceptance test" alone |
| **draft workplan** | Stub created by T3 auto-invoke at `docs/workplans/drafts/draft-*.json`. No worktrees/agents/specs until promoted. | "scaffold workplan" |
| **task tier** | Classification by `hex hook route`: T1 Todo (silent), T2 Mini-plan (one-line hint), T3 Workplan (auto-draft). ADR-2026-04-11-0227. | "priority" alone |

## Deployment Units

| Term | Definition | NOT |
|------|-----------|-----|
| **deployment unit** | One of the 5 things that ship/run independently: SpacetimeDB, hex-nexus, hex-agent, hex-dashboard, hex clients. | "service", "component" alone |
| **hex client** | Anything that drives hex remotely: hex-cli, hex-desktop, web app, chat. | "frontend" alone |
| **fleet** | Set of hex agents reporting to a single hex-nexus across multiple machines. | "swarm" (HexFlo-specific) |

## Banned / Deprecated

| Term | Why banned | Use instead |
|------|-----------|-------------|
| **hex-hub** | Renamed to hex-nexus | hex-nexus |
| **ruflo** | Replaced by HexFlo | HexFlo |
| **hub.db** | SQLite fallback removed; STDB-only | (no replacement — file no longer exists) |
| **claude-flow** | Internal coordination is HexFlo, not claude-flow | HexFlo |
| **hex-chat** | Crate removed | (use chat-relay WASM module + dashboard) |

## Notes on Enforcement

- **Phase 4** of ADR-047 will add `hex docs check` to scan reference docs against this file and flag terminology drift. Until then, enforcement is manual (PR review + adr-reviewer agent).
- **Glossary updates** require an ADR amendment if they change a canonical term — drift in the glossary itself is the worst kind of drift.
- **Component additions**: when a new deployment unit or WASM module ships, it MUST be added here in the same commit.

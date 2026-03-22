# ADR-052: AIIDE — Hex Nexus as AI Integrated Development Environment

**Status:** Accepted
**Date:** 2026-03-21
**Drivers:** Dashboard redesign, OpenCode UX research, agent fleet management needs

## Context

Hex Nexus evolved from a monitoring dashboard into something that doesn't fit existing categories. It is not an IDE with AI features (Cursor, Copilot). It is not a chat app with code execution (ChatGPT). It is not a DevOps dashboard (Grafana). It is a purpose-built environment where AI agents are the primary developers and humans provide guidance.

We coin the term **AIIDE** (AI Integrated Development Environment, pronounced "aide") to describe this new category.

## Decision

Hex Nexus is an AIIDE. Its design principles are:

### 1. Five Pillars
- **Projects** — isolated worktrees, hex architecture analysis, dependency graphs
- **Agents** — local (Claude Code, hex-coder) and remote (hex-agent over SSH), with lifecycle management
- **Swarms** — HexFlo multi-agent coordination with task tracking and progress visualization
- **Inference** — multi-provider management (Ollama, OpenAI, Anthropic) with health, cost, token budget
- **Configuration** — architecture blueprints, MCP tools, hooks, skills, CLAUDE.md context, agent definitions

### 2. Navigation Model
Breadcrumb-based hierarchical navigation:
```
Control Plane
  ├── Project: {name}
  │   ├── Chat: {session} [Plan|Build]
  │   ├── ADR: {number}
  │   ├── Health Analysis
  │   └── Dependency Graph
  ├── Agent Fleet
  ├── Configuration
  │   ├── Architecture Blueprint
  │   ├── MCP Tools
  │   ├── Hooks
  │   ├── Skills
  │   ├── Context (CLAUDE.md)
  │   ├── Agent Definitions
  │   └── SpacetimeDB
  ├── Inference
  └── Fleet Nodes
```

### 3. State Architecture
- SpacetimeDB is the single source of truth for ALL coordination state (ADR-042)
- hex-nexus binary is stateless compute (filesystem, processes, outbound HTTP)
- Dashboard connects directly to SpacetimeDB via WebSocket subscriptions
- Changes from any source (CLI, MCP, another AIIDE session) propagate in real-time

### 4. Chat Scoping
- Control Plane chat = manage infrastructure ("create a swarm", "add a project")
- Project chat = develop code scoped to that project ("fix the auth adapter")
- Plan mode (blue) = discuss, no side effects
- Build mode (green) = execute changes

### 5. Design System
- Dark mode default (bg: #0a0e14)
- Three-column layout: sidebar (272px) | center (flexible) | context panel (320px)
- JetBrains Mono for code/identifiers, Inter for UI text
- Hex layer colors: Domain=#58a6ff, Ports=#bc8cff, UseCases=#3fb950, Primary=#f0883e, Secondary=#d29922
- Status colors: green=active/healthy, cyan=in-progress, yellow=warning/current, red=error/failed, gray=idle

## Consequences

**Positive:**
- Clear product identity and category definition
- Complete information architecture covering all developer needs
- Every feature accessible via breadcrumbs, sidebar, or command palette
- Real-time state via SpacetimeDB eliminates polling and stale data
- Agents are first-class citizens with visibility into local and remote fleets

**Negative:**
- Large surface area to implement (18 pages)
- SpacetimeDB dependency is now mandatory
- Design system must be maintained across all views

**References:**
- AIIDE Vision Document: `docs/architecture/aiide-vision.md`
- Pencil Design File: 8 screens designed with full layout specs
- UX Research: `docs/analysis/ux-research-competitor-analysis.md` (9 tools analyzed)
- ADR-042: SpacetimeDB as Single Source of State
- ADR-039: Nexus Agent Control Plane

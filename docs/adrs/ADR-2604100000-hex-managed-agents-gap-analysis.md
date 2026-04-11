# ADR-2604100000: hex Managed Agents Gap Analysis vs Anthropic Claude Managed Agents

**Status:** Accepted
**Date:** 2026-04-10
**Drivers:** User asked how far hex is from achieving feature parity with Anthropic Managed Agents. This ADR identifies gaps and specifies closing them.
**Supersedes:** ADR-2603282000 (partially — builds on Docker sandbox foundation), ADR-2603291900 (partially — builds on worker execution)

## Context

Anthropic released [Claude Managed Agents](https://platform.claude.com/docs/en/managed-agents/overview) — a pre-built agent harness that runs in managed cloud infrastructure. The user asked how far hex is from achieving feature parity.

### Core Managed Agents concepts

| Concept | Description |
|--------|-------------|
| **Agent** | Model + system prompt + tools + MCP servers + skills |
| **Environment** | Container templates with packages, network access |
| **Session** | Running instance within an environment, performing a task |
| **Events** | Messages exchanged via Server-Sent Events (SSE) streaming |

### Feature comparison

| Feature | Anthropic Managed Agents | hex (current) | Gap |
|---------|------------------------|--------------|-----|
| **Agent definition** | API-created, stored server-side | hex-agent YAML (14 agents) | Small — YAML needs API wrapper |
| **Environment** | Container templates via API | Dockerfile + Docker Sandbox | Medium — needs API for env templates |
| **Session** | Server-side, SSE streaming | HexFlo task in SpacetimeDB | Medium — needs real-time SSE |
| **File operations** | Built-in `Write`, `Read`, `Edit` | hex MCP tools via `hex_batch_execute` | Small — needs native file tools |
| **Bash/execute** | Built-in | Agent tool | **Large — no native bash** |
| **Web search/fetch** | Built-in | `ctx_*` MCP via plugin | **Large — external dependency** |
| **MCP servers** | First-class via config | hex MCP (mcp-tools.json) | Small — needs integration |
| **Multi-agent** | Research preview | HexFlo swarm (ADR-027) ✅ | Small — ours is production |
| **Memory** | Research preview | HexFlo memory tools ✅ | Small — ours is production |
| **Container isolation** | Docker microVMs | ADR-2603282000 Docker Sandbox | **Medium — not wired** |
| **Long-running** | Cloud containers | Background agents | **Large — host-bound** |
| **State persistence** | Server-side | SpacetimeDB | **Medium — polling, not SSE** |
| **Steering/interrupt** | Send events to guide agent mid-execution | None | **Large — missing** |
| **Outcomes** | Define success criteria | None | **Large — missing** |
| **Tool allowlists** | Built-in safety | hex-agent MCP gateway | Small — we have this |

### Gaps that require new work

1. **SSE streaming** — hex-nexus uses REST polling, not Server-Sent Events for real-time updates
2. **Native bash tool** — hex relies on Claude Code Agent tool, not a native `bash` MCP
3. **Web search/fetch tools** — hex delegates to `plugin:context-mode`, not native
4. **Docker worker wiring** — ADR-2603291900 phase P4 (the `if false &&` guard) is not done
5. **Steering API** — no way to send events to guide a running agent mid-execution
6. **Outcomes** — no declarative success criteria system (Managed Agents research preview feature)
7. **Environment API** — no HTTP API to create/query/delete container environments
8. **Session API** — no HTTP API to create/query/stream session status

### What hex already has that Managed Agents doesn't

1. **Architecture enforcement** — hexagonal boundary checks at mutation point
2. **ADR governance** — design decisions as code
3. **Swarm coordination** — production-native HexFlo, not research preview
4. **Local-first** — no cloud dependency required
5. **Inference routing** — model selection per complexity (ADR-2603271000)

## Decision

We will close the gap between hex and Anthropic Managed Agents by implementing the following improvements, prioritized by impact:

### 1. SSE streaming for real-time sessions (P0 — highest)

Replace REST polling with Server-Sent Events for:
- Task assignment notifications
- Agent heartbeat updates
- Session status streaming
- Progress updates to dashboard

Implementation:
- Add `axum` SSE handler in hex-nexus: `GET /api/sessions/{id}/stream`
- Subscribe to SpacetimeDB via `subscribe` reducer
- Stream events as they happen

### 2. Native hex-bash MCP tool (P1 — high)

Expose a `hex_bash` MCP tool that:
- Accepts command string + optional timeout
- Runs allowlisted commands only (configurable in `.hex/config.toml`)
- Blocks dangerous commands: `rm -rf /`, `git push --force`, network calls outside policy
- Returns stdout/stderr/exit code

Replaces: Claude Code `Bash` tool for Docker path

### 3. Native hex-web MCP tools (P1 — high)

Implement `hex_web_search` and `hex_web_fetch` as native MCP tools:
- Uses `reqwest` for HTTP calls (no external dependency)
- Configurable allowlist of domains
- Respects `.hex/config.toml` network policy

Replaces: `plugin:context-mode` tools

### 4. Complete Docker worker delegation (P2 — medium)

Complete ADR-2603291900:
- P0: Enrich task metadata with WorkplanStep JSON + output_dir
- P1: Implement real `hex-coder` worker
- P2: Supervisor reads worker result from hexflo memory
- P3: Set HEX_OUTPUT_DIR env var
- **P4: Remove `if false &&` guard** — enables worker delegation

### 5. Steering API (P3 — lower)

Add ability to send events to a running session:
- `POST /api/sessions/{id}/events` — send event to running agent
- `POST /api/sessions/{id}/interrupt` — stop current tool execution, restart with new event
- Agent reads pending events via SSE subscription

This enables: human-in-the-loop guidance during long-running tasks

### 6. Outcomes (P4 — lower)

Declarative success criteria (Managed Agents research preview):
- Define outcomes in workplan: ` outcomes: [{ id, evaluate, required }]`
- During execution, evaluate outcome after each phase
- Stream outcome results via SSE for real-time visibility

This is lower priority — hex already has objective evaluation in pipeline

### 7. Environment/Session API (P5 — nice to have)

HTTP API for container and session management:
- `POST /api/environments` — create environment from template
- `GET /api/environments` — list environments
- `POST /api/sessions` — create session with agent + environment
- `GET /api/sessions/{id}` — get session status
- `GET /api/sessions/{id}/stream` — SSE stream

This is lower priority — CLI-first is hex's primary interface

## Implementation phases

| Phase | Description | Priority | Status |
|-------|-------------|----------|--------|
| P0a | hex-nexus SSE handler for real-time events | High | ✅ Done (stdb-agent-workflow) |
| P0b | SpacetimeDB subscription for SSE events | High | ✅ Done (stdb-agent-workflow) |
| P1a | `hex_bash` MCP tool with allowlist | High | ✅ Done (managed-agents-autonomy) |
| P1b | `hex_web_search` MCP tool | High | ✅ Done (managed-agents-autonomy) |
| P1c | `hex_web_fetch` MCP tool | High | ✅ Done (managed-agents-autonomy) |
| P2a | ADR-2603291900 P0: task metadata enrichment | Medium | ✅ Done |
| P2b | ADR-2603291900 P1: real hex-coder worker | Medium | ✅ Done |
| P2c | ADR-2603291900 P4: remove `if false &&` guard | Medium | ✅ Done |
| P3a | Steering API: `POST /api/sessions/{id}/events` | Lower | ✅ Done |
| P3b | Steering API: `POST /api/sessions/{id}/interrupt` | Lower | ✅ Done |
| P4a | Outcomes in workplan JSON | Lower | ✅ Done (gates in JSON) |
| P4b | Outcome evaluation during pipeline | Lower | ✅ Done (gate results) |
| P5a | Environment CRUD API | Nice | ✅ Done |
| P5b | Session CRUD API | Nice | ✅ Done |
| P5c | Session SSE streaming | Nice | ✅ Done |

## Consequences

**Positive:**
- Feature parity with Anthropic Managed Agents (except cloud infrastructure)
- Native tools reduce external dependencies
- SSE streaming provides real-time visibility
- Docker workers become first-class (ADR-2603291900 completes)
- Steering enables human-in-the-loop control

**Negative:**
- Additional implementation effort — ~3 months of work
- SSE adds complexity to hex-nexus
- More MCP tools to maintain

**Mitigations:**
- Phased rollout: SSE first, then native tools, then worker delegation
- Each phase is independently testable

## References

- ADR-2603282000: Docker Sandbox Agent Coordination
- ADR-2603291900: Docker Worker First-Class Execution
- ADR-2603271000: Quantization-aware inference routing
- ADR-2603271000: Secure inference and secrets
- https://platform.claude.com/docs/en/managed-agents/overview — Anthropic Managed Agents docs
- hex-cli/assets/mcp/mcp-tools.json — existing MCP tool definitions
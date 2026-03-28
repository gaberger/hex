# ADR-2603282000: hex-agent as Claude Code-Independent Runtime in Docker AI Sandbox

**Status:** Proposed
**Date:** 2026-03-28
**Drivers:** Claude Code dependency limits model choice and portability; agents run without isolation on the host filesystem; no enforcement of worktree boundaries; no credential scoping per agent
**Supersedes:** None

## Context

hex currently depends on Claude Code as its agent runtime. This creates several problems:

1. **Model lock-in** — Claude Code only uses Anthropic models. hex has a working inference gateway that can route to Ollama (local), OpenRouter (frontier + free), and Anthropic direct, but nothing uses it for agent execution.
2. **No isolation** — background agents run directly on the host filesystem, editing files on `main`, conflicting with other sessions
3. **No enforcement at mutation point** — architecture boundary checks run at analysis time (`hex analyze`), not at the moment a file is written
4. **No worktree enforcement** — ADR-004 mandates worktree isolation but nothing enforces it; agents bypass it regularly
5. **No credential scoping** — any agent can use any key from the vault; there is no per-task credential grant enforcement at the runtime level

Docker AI Sandboxes (https://docs.docker.com/ai/sandboxes/) provides exactly the infrastructure needed:
- **microVMs** — each agent gets its own lightweight VM with a private Docker daemon; processes and network are isolated from the host
- **MCP Gateway** — orchestrates tool access by running MCP servers as Docker containers; injects credentials on demand; logs all tool calls
- **Network policy** — default-deny with explicit allow-lists; agents can only reach SpacetimeDB, hex-nexus, and inference endpoints
- **Docker Model Runner** — local LLM execution, complementing the existing Ollama/vLLM inference path

## Decision

### 1. hex-agent becomes a Claude Code-independent agent runtime

hex-agent shall operate as a self-contained agentic loop:

```
HexFlo task (via SpacetimeDB subscription)
  → hex-agent selects model via inference gateway
  → LLM generates tool calls
  → hex-agent enforces hex architecture on every tool call
  → hex-agent executes (file write, git op, etc.)
  → progress reported back to SpacetimeDB
  → repeat until task complete
```

hex-agent is **not a Claude Code skill or hook runner** — it is the runtime itself. Claude Code remains usable for interactive development via `--no-sandbox`, but swarm agents use hex-agent exclusively.

### 2. hex-agent runs inside a Docker AI Sandbox microVM

Each swarm agent gets its own microVM via Docker AI Sandboxes:

- **Filesystem isolation** — the microVM bind-mounts only its assigned git worktree at `/workspace`; no access to host `~/.hex/`, project root, or other worktrees
- **Private Docker daemon** — the agent can spin up test containers inside its VM without affecting the host
- **Network policy** (default-deny, explicit allows via `docker sandbox network proxy` or sandbox YAML):
  ```yaml
  network:
    allow:
      - host.docker.internal:3033   # SpacetimeDB WebSocket (macOS/Docker Desktop)
      - host.docker.internal:5555   # hex-nexus REST API
      - bazzite:11434               # Ollama local inference
      - openrouter.ai:443           # OpenRouter frontier/free models
      - crates.io:443               # Rust dependencies
      - static.crates.io:443
      - npmjs.com:443               # Node dependencies
  ```
  On Linux, `host.docker.internal` is not available by default — nexus injects `SPACETIMEDB_HOST` with the host bridge IP (`172.17.0.1`) or the host's actual LAN IP at spawn time.

  **SpacetimeDB is already configured correctly** — `hex nexus start` launches SpacetimeDB with `--listen-addr 0.0.0.0:3033`, making it reachable from microVMs without any additional configuration.

### 3. hex-agent exposes an MCP server with architecture-enforcing tools

Inside the microVM, hex-agent runs as an **MCP server** providing hex-aware tools to the LLM:

| Tool | Replaces | Enforcement |
|------|----------|-------------|
| `hex_write_file` | Claude Code `Write` | Validates target path is inside `/workspace`; checks hex boundary rules before writing |
| `hex_edit_file` | Claude Code `Edit` | Same as write + validates import additions don't cross layer boundaries |
| `hex_read_file` | Claude Code `Read` | Unrestricted read within `/workspace` |
| `hex_bash` | Claude Code `Bash` | Allowlist of safe commands; blocks `git push`, `rm -rf /`, network calls outside policy |
| `hex_git_commit` | `git commit` via Bash | Enforces commit message format, runs `cargo check` / `bun test` before committing |
| `hex_git_status` | `git status` via Bash | Read-only, unrestricted |
| `hex_analyze` | `hex analyze .` | Runs boundary analysis; returns violations as structured data to LLM |
| `hex_inference` | Direct LLM call | Routes through inference gateway — model selection per complexity score |

The LLM never calls the host filesystem directly — all mutations go through hex-agent's MCP tools.

### 4. SpacetimeDB WebSocket coordination from inside the microVM

The hex-agent binary connects to SpacetimeDB at startup:

```
SPACETIMEDB_HOST=ws://<host>:3033   # injected by nexus at spawn
SPACETIMEDB_TOKEN=<per-agent token> # scoped identity, auto-expires
HEX_AGENT_ID=<assigned by nexus>
HEXFLO_TASK=<task id>
```

Operations:
1. Register agent via `register_agent` reducer
2. Heartbeat every 30s via `agent_heartbeat` reducer
3. Subscribe to `swarm_task` filtered to this `agent_id` for assignments
4. Call `task_complete` reducer when done
5. `memory_store` / `memory_retrieve` for cross-agent coordination

### 5. Credential injection via Docker MCP Gateway

The Docker AI Sandbox MCP Gateway injects credentials on demand from the hex vault:
- `ANTHROPIC_API_KEY` / `OPENROUTER_API_KEY` — for inference calls
- `SPACETIMEDB_TOKEN` — scoped per-agent, expires on container exit
- No credentials are written to the microVM filesystem

Secret grants follow ADR-2603261000: the spawning session must hold a `hex secrets grant` before secrets are injected.

### 6. Model selection — any model, no Claude Code dependency

hex-agent selects the LLM for each inference call via the existing inference gateway (ADR-2603271000):
- **Low complexity tasks** → local Ollama model (e.g. `qwen3.5:9b`)
- **High complexity tasks** → frontier via OpenRouter (e.g. `anthropic/claude-opus-4.5`, `deepseek/deepseek-v3.2`)
- **Escalation** — if local model produces failing tests after N iterations, escalate to next tier

This makes hex completely model-agnostic. Claude Code remains available for interactive use (`--no-sandbox`) but is not required.

### 7. Worktree enforcement gate (non-Docker path)

For interactive Claude Code sessions that don't use Docker, `hex hook subagent-start` shall hard-error if `HEXFLO_TASK` is set and the current working directory is the project root (not a worktree). This prevents agents from accidentally running on `main` when Docker is unavailable.

## Consequences

**Positive:**
- Model-agnostic — any LLM via inference gateway; not locked to Anthropic
- Architecture enforcement at mutation point — `hex_write_file` validates before writing
- Strong filesystem isolation — microVM; worktree collision is impossible
- Credential scoping — per-agent tokens, never stored on disk
- Portable — hex-agent can run on any machine with Docker, no Claude Code license required
- Full audit trail — MCP Gateway logs every tool call

**Negative:**
- Significant implementation effort — hex-agent needs an agentic loop, MCP server, LLM client
- Docker AI Sandboxes is a relatively new feature — API may change
- microVM startup is slower than plain container (~3-5s vs ~1s)
- LLM quality for code generation may differ from Claude Code depending on model selected

**Mitigations:**
- Phased rollout: MCP server first, agentic loop second, Docker Sandbox third
- `--no-sandbox` for Claude Code interactive path remains fully supported
- Inference gateway escalation ensures quality floor is maintained

## Implementation

| Phase | Description | Status |
|-------|------------|--------|
| P1 | `hex-agent`: MCP server exposing `hex_write_file`, `hex_edit_file`, `hex_read_file`, `hex_bash`, `hex_git_commit`, `hex_analyze` | Pending |
| P2 | `hex-agent`: boundary enforcement in `hex_write_file` / `hex_edit_file` — validate import paths before writing | Pending |
| P3 | `hex-agent`: agentic loop — receive task from SpacetimeDB, call LLM, execute tool calls, report completion | Pending |
| P4 | `hex-agent/Dockerfile` + Docker AI Sandbox config — microVM definition, network policy, MCP Gateway setup | Pending |
| P5 | `hex-nexus`: `POST /api/agents/spawn` — launch microVM, inject env vars, return `{container_id, agent_id}` | Pending |
| P6 | `hex hook subagent-start` — hard-error gate for non-Docker path (HEXFLO_TASK + cwd == project root) | Pending |
| P7 | `hex swarm` — default to microVM spawn; `--no-sandbox` for Claude Code interactive path | Pending |
| P8 | `hex test coordination` — spawn two agents in parallel, verify no filesystem collision, verify heartbeats in SpacetimeDB | Pending |

## References

- ADR-004: Worktree isolation for feature development
- ADR-027: HexFlo swarm coordination
- ADR-048: Task state synchronisation via subagent hooks
- ADR-058: Unified agent identity
- ADR-2603261000: Secure inference and secrets
- ADR-2603271000: Quantization-aware inference routing
- https://docs.docker.com/ai/sandboxes/ — Docker AI Sandboxes

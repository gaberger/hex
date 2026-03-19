# ADR-037: Agent Lifecycle — Local Default + Remote Connect

- **Status**: Proposed
- **Date**: 2026-03-19
- **Informed by**: ADR-024, ADR-035, VS Code Remote SSH model
- **Authors**: Gary (architect), Claude (analysis)

## Context

hex-nexus is the coordination plane for AI agent fleets, but currently no agent starts automatically. The chat dashboard can talk to Ollama directly via the LLM bridge, but this is a simple prompt→response loop — no tool use, no filesystem access, no architecture awareness.

For hex-chat to be a true "developer command center," it needs an agent that can:
1. Read/write files in the project
2. Execute shell commands
3. Use hex tools (analyze, summarize, scaffold)
4. Participate in workplan execution

Two usage patterns exist:

| Pattern | Example | Need |
|---|---|---|
| **Solo developer** | Mac laptop, local project | Agent runs locally, immediate access |
| **Fleet operator** | Mac control plane, bazzite GPU, cloud agents | Agents connect remotely to central nexus |

VS Code solved this elegantly: a local server starts by default, but `Remote - SSH` lets you connect to any machine. The agent runs WHERE THE CODE IS, the UI runs where the developer is.

## Decision

### 1. Default Local Agent

When `hex nexus start` runs, it automatically spawns a local `hex-agent` instance that:
- Registers with nexus via WebSocket (`/ws/chat`)
- Has access to the current working directory
- Uses the registered inference provider (Ollama/Anthropic) for LLM calls
- Runs as a background process tied to the nexus lifecycle

```
hex nexus start
  ⬡ hex-nexus started (PID 1234, port 5555)
  ⬡ hex-chat web started (PID 1235) at http://127.0.0.1:5556
  ⬡ hex-agent started (PID 1236) — project: /path/to/cwd
```

The default agent is **opt-out**, not opt-in:
```
hex nexus start --no-agent    # skip default agent
```

### 2. Remote Agent Connect

Any `hex-agent` instance can connect to any nexus:

```bash
# On bazzite (GPU box with local models):
hex-agent --remote ws://mac.local:5555/ws/chat \
          --project-dir /path/to/project \
          --model qwen3.5:27b
```

The agent:
1. Connects via WebSocket to the nexus
2. Sends `AgentRegister` message with name, project_dir, capabilities
3. Receives chat messages and tool requests
4. Executes tools locally (on the machine where the agent runs)
5. Sends results back via WebSocket

### 3. Agent Registry

Nexus maintains a registry of connected agents:

```
GET /api/agents → [
  { id: "local-1", name: "default", project_dir: "/Users/gary/project", status: "running", remote: false },
  { id: "bazzite-1", name: "bazzite-gpu", project_dir: "/home/gary/project", status: "running", remote: true, host: "bazzite.local" }
]
```

### 4. Agent Routing

When a user sends a chat message:
1. If `@agent-name` prefix → route to specific agent
2. If a local agent is connected → route to it (default)
3. If only remote agents → route to first available
4. If no agents → use LLM bridge (direct Ollama/Anthropic, no tools)

### 5. Agent Discovery (hex-agent side)

`hex-agent` needs a `--remote` flag that:
1. Connects via WebSocket (not HTTP polling)
2. Handles `chat_message` events as input
3. Sends `stream_chunk`, `tool_call`, `tool_result` events as output
4. Sends heartbeats every 15 seconds
5. Reconnects on disconnect with exponential backoff

### 6. Lifecycle Management

| Event | Behavior |
|---|---|
| `hex nexus start` | Spawn default local agent |
| `hex nexus stop` | Kill all local agents (remote agents disconnect gracefully) |
| Agent crash | Nexus marks agent as `dead` after 120s without heartbeat |
| Agent reconnect | Re-registers, resumes pending tasks |
| `hex agent list` | Show all connected agents |
| `hex agent kill <id>` | Terminate specific agent |

## Consequences

### Positive
- Zero-config experience: `hex nexus start` gives you a working agent immediately
- Fleet scalability: add agents on GPU boxes without changing the nexus
- Code stays where it runs: agents execute tools locally, not remotely
- Multiple agents can work on the same project in parallel (different worktrees)

### Negative
- Default agent adds ~50MB memory overhead (hex-agent process)
- Remote agents need network access to nexus (firewall considerations)
- Agent routing adds complexity to the chat WS handler

### Risks
- Agent process management across platforms (PID files, signal handling)
- Remote agent security: need auth tokens for remote connections
- Tool execution isolation: remote agents run arbitrary commands on their host

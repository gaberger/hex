# ADR-040: Remote Agent Transport — WebSocket over SSH with SpacetimeDB Coordination

- **Status**: Proposed
- **Date**: 2026-03-21
- **Informed by**: ADR-025 (SpacetimeDB), ADR-037 (agent lifecycle), ADR-039 (control plane)
- **Authors**: Gary (architect), Claude (analysis)

## Context

ADR-037 established the local-default / remote-connect agent lifecycle model. ADR-039 described the nexus control plane vision. What's missing is the **transport layer** — how remote agents actually communicate with nexus securely, and how SpacetimeDB coordinates agent state across machines.

### Current State

| Component | Status | Gap |
|-----------|--------|-----|
| Agent registry | Built (HexFlo) | Local only — no cross-network discovery |
| WebSocket chat | Built (`/ws/chat`) | No SSH tunneling, no auth beyond local |
| SpacetimeDB bindings | Built (10 modules) | Not wired for remote agent coordination |
| SSH routes | Partial (`/api/fleet/ssh`) | Shell-out to `ssh`, no tunnel management |
| Inference routing | Built (Ollama/OpenAI/vLLM) | No remote inference server brokering |

### Problem

A developer on their Mac wants to:
1. Run `hex nexus start` locally (control plane + dashboard)
2. Have a bazzite GPU box auto-register as a remote agent with access to local models
3. Route code generation tasks to whichever agent has capacity
4. Stream partial results back in real-time
5. All secured via SSH — no exposed ports, no VPN required

This requires three things that don't exist yet:
- **Secure transport**: WebSocket connections tunneled through SSH
- **Coordination**: SpacetimeDB as the single source of truth for agent state, task assignment, and inference server availability
- **Routing**: Smart request routing based on agent capacity, model availability, and network latency

## Decision

### 1. Transport Architecture

```
┌──────────────────────┐         SSH Tunnel          ┌──────────────────────┐
│  hex-nexus (Mac)     │◄════════════════════════════►│  hex-agent (bazzite) │
│                      │    WebSocket inside tunnel    │                      │
│  ┌────────────────┐  │                              │  ┌────────────────┐  │
│  │ WS Acceptor    │◄─┼──────── wss://tunnel ───────►│  │ WS Client      │  │
│  │ (axum)         │  │                              │  │ (tungstenite)  │  │
│  └───────┬────────┘  │                              │  └───────┬────────┘  │
│          │           │                              │          │           │
│  ┌───────▼────────┐  │                              │  ┌───────▼────────┐  │
│  │ SpacetimeDB    │◄─┼──── subscription sync ──────►│  │ SpacetimeDB    │  │
│  │ Client         │  │                              │  │ Client         │  │
│  └────────────────┘  │                              │  └────────────────┘  │
└──────────────────────┘                              └──────────────────────┘
```

**Three communication channels:**

| Channel | Purpose | Protocol |
|---------|---------|----------|
| **SSH Tunnel** | Secure transport, no exposed ports | `russh` (pure Rust SSH2) |
| **WebSocket** | Bidirectional streaming (chat, tool calls, results) | `tokio-tungstenite` over SSH tunnel |
| **SpacetimeDB** | State sync, agent registry, task coordination | Native WebSocket subscriptions |

### 2. SSH Tunnel Management

Use `russh` (pure Rust, async, no OpenSSH dependency) for programmatic tunnel control:

```rust
// Port: SSH tunnel lifecycle
pub trait ISshTunnelPort: Send + Sync {
    /// Establish SSH tunnel to remote host, return local forwarded port
    async fn connect(&self, config: SshTunnelConfig) -> Result<TunnelHandle>;
    /// Check tunnel health
    async fn health(&self, handle: &TunnelHandle) -> TunnelHealth;
    /// Reconnect with exponential backoff
    async fn reconnect(&self, handle: &TunnelHandle) -> Result<()>;
    /// Tear down tunnel
    async fn disconnect(&self, handle: &TunnelHandle) -> Result<()>;
    /// List active tunnels
    async fn list_tunnels(&self) -> Vec<TunnelInfo>;
}

pub struct SshTunnelConfig {
    pub host: String,
    pub port: u16,                    // default 22
    pub user: String,
    pub auth: SshAuth,                // Key, Agent, or Password
    pub remote_bind_port: u16,        // nexus WS port on remote (5555)
    pub local_forward_port: u16,      // local port to forward through
    pub keepalive_interval_secs: u16, // default 15
    pub reconnect_max_attempts: u8,   // default 5
}

pub enum SshAuth {
    Key { path: PathBuf, passphrase: Option<String> },
    Agent,  // ssh-agent forwarding
}
```

**Why `russh` over shelling out to `ssh`:**
- Programmatic tunnel lifecycle (no PID tracking, no zombie processes)
- Async-native — integrates with tokio runtime
- Key/agent auth without spawning processes
- Health checks without parsing `ssh` output

### 3. WebSocket Protocol (over SSH tunnel)

Extend the existing `/ws/chat` protocol with agent-specific message types:

```rust
#[derive(Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum AgentMessage {
    // Registration
    Register { agent_id: String, capabilities: AgentCapabilities, project_dir: String },
    RegisterAck { session_id: String },

    // Heartbeat
    Ping { timestamp: u64 },
    Pong { timestamp: u64 },

    // Task assignment (nexus → agent)
    TaskAssign { task_id: String, request: CodeGenRequest },
    TaskCancel { task_id: String, reason: String },

    // Results (agent → nexus)
    StreamChunk { task_id: String, chunk: String, sequence: u32 },
    TaskComplete { task_id: String, result: CodeGenResult },
    TaskFailed { task_id: String, error: String },

    // Tool execution (bidirectional)
    ToolCall { call_id: String, tool: String, args: serde_json::Value },
    ToolResult { call_id: String, output: serde_json::Value, error: Option<String> },

    // Inference routing
    InferenceRequest { request_id: String, model: String, prompt: String, params: InferenceParams },
    InferenceChunk { request_id: String, token: String },
    InferenceComplete { request_id: String, full_response: String, usage: TokenUsage },
}

pub struct AgentCapabilities {
    pub models: Vec<String>,          // ["qwen3.5:27b", "codestral:22b"]
    pub tools: Vec<String>,           // ["fs", "shell", "hex-analyze", "hex-generate"]
    pub max_concurrent_tasks: u8,
    pub gpu_vram_mb: Option<u32>,     // for inference routing decisions
}
```

### 4. SpacetimeDB Coordination Tables

New WASM module tables for remote agent coordination:

```rust
// spacetime_module/src/remote_agents.rs

#[spacetimedb::table(public, name = remote_agents)]
pub struct RemoteAgent {
    #[primary_key]
    pub agent_id: String,
    pub name: String,
    pub host: String,
    pub project_dir: String,
    pub status: String,        // "connecting" | "online" | "busy" | "stale" | "dead"
    pub capabilities_json: String,
    pub last_heartbeat: u64,
    pub connected_at: u64,
    pub tunnel_id: Option<String>,
}

#[spacetimedb::table(public, name = inference_servers)]
pub struct InferenceServer {
    #[primary_key]
    pub server_id: String,
    pub agent_id: String,      // which agent provides this
    pub provider: String,      // "ollama" | "vllm" | "llama-cpp"
    pub base_url: String,
    pub models_json: String,   // ["qwen3.5:27b", "codestral:22b"]
    pub gpu_vram_mb: u32,
    pub status: String,        // "available" | "busy" | "offline"
    pub current_load: f32,     // 0.0 - 1.0
}

#[spacetimedb::table(public, name = code_gen_tasks)]
pub struct CodeGenTask {
    #[primary_key]
    pub task_id: String,
    pub swarm_id: Option<String>,
    pub assigned_agent_id: Option<String>,
    pub status: String,        // "pending" | "assigned" | "streaming" | "complete" | "failed"
    pub request_json: String,
    pub result_json: Option<String>,
    pub created_at: u64,
    pub assigned_at: Option<u64>,
    pub completed_at: Option<u64>,
}

// Reducers
#[spacetimedb::reducer]
pub fn register_remote_agent(ctx: &ReducerContext, agent: RemoteAgent) { ... }

#[spacetimedb::reducer]
pub fn update_agent_heartbeat(ctx: &ReducerContext, agent_id: String) { ... }

#[spacetimedb::reducer]
pub fn assign_task(ctx: &ReducerContext, task_id: String, agent_id: String) { ... }

#[spacetimedb::reducer]
pub fn complete_task(ctx: &ReducerContext, task_id: String, result: String) { ... }

#[spacetimedb::reducer]
pub fn reassign_task(ctx: &ReducerContext, task_id: String, reason: String) { ... }
```

### 5. Request Routing Strategy

The orchestrator use case routes code generation requests based on:

```
Priority 1: Model availability — does the agent have the requested model loaded?
Priority 2: Current load — pick the least-loaded agent (from SpacetimeDB current_load)
Priority 3: Network locality — prefer local agent for small tasks, remote GPU for large generation
Priority 4: Affinity — if agent already has project context cached, prefer it
```

Fallback chain:
```
Remote GPU agent (has model, low load)
  → Local agent (slower but available)
    → Direct LLM bridge (no tools, prompt→response only)
      → Error: no agents available
```

### 6. Security Model

| Concern | Solution |
|---------|----------|
| Network exposure | SSH tunnel — no exposed ports on remote machines |
| Authentication | SSH key-based auth (no passwords in production) |
| Authorization | Agent capabilities declare what tools are permitted |
| Secret forwarding | Encrypted over SSH tunnel, never stored on remote |
| Tunnel hijacking | Per-session nonce in `RegisterAck`, validated on every message |
| SpacetimeDB access | Token-scoped access; agents can only modify their own rows |

### 7. CLI Commands

```bash
# Connect to a remote nexus (run on remote machine)
hex agent connect ws://mac.local:5555 --ssh-key ~/.ssh/id_ed25519

# Connect with SSH tunnel (run from control plane)
hex agent spawn-remote user@bazzite.local --project-dir /path/to/project

# List all agents (local + remote)
hex agent list

# Show agent detail
hex agent info <agent-id>

# Disconnect a remote agent
hex agent disconnect <agent-id>

# Route a task to a specific agent
hex agent route <task-id> --to <agent-id>
```

### 8. One-Command Remote Deploy (`hex agent spawn-remote`)

**Validated 2026-03-21**: hex-agent on bazzite.local successfully connected to Mac's nexus
via SSH reverse tunnel, registered as `hex-swift-prism-57d3`, and entered hub-managed mode
with 45 skills loaded. The manual process was:

```
1. rsync source to bazzite
2. cargo build on bazzite
3. ssh -f -N -R 5555:127.0.0.1:5555 bazzite.local
4. ssh bazzite.local "hex-agent --hub-url http://127.0.0.1:5555 ..."
```

This must become one command: `hex agent spawn-remote gary@bazzite.local`

#### Spawn Protocol

When the operator runs `hex agent spawn-remote user@host`, nexus performs:

```
Phase 1: PROVISION
  ├─ SSH connect to user@host (russh, key or agent auth)
  ├─ Check if hex-agent binary exists at ~/.hex/bin/hex-agent
  ├─ If missing or outdated: scp the binary from nexus host
  │   └─ Cross-compile target selection: detect remote arch via `uname -m`
  │      ├─ x86_64 → linux-x86_64 binary
  │      └─ aarch64 → linux-aarch64 binary
  └─ Verify binary: ssh run `~/.hex/bin/hex-agent --build-hash`

Phase 2: TUNNEL
  ├─ Establish SSH reverse tunnel: remote:$AGENT_PORT → localhost:$NEXUS_PORT
  │   └─ Uses russh channel_open_direct_tcpip (programmatic, no ssh CLI)
  └─ Verify tunnel: ssh run `curl -s http://127.0.0.1:$AGENT_PORT/api/version`

Phase 3: LAUNCH
  ├─ SSH exec: `~/.hex/bin/hex-agent --hub-url http://127.0.0.1:$AGENT_PORT
  │     --hub-token $SESSION_TOKEN --project-dir $PROJECT_DIR --no-preflight`
  ├─ Wait for WebSocket Register message (30s timeout)
  ├─ Send RegisterAck with session nonce
  └─ Start heartbeat monitor

Phase 4: CONFIRM
  ├─ Agent appears in `hex agent list`
  ├─ Nexus dashboard shows remote agent with host + models
  └─ Agent is ready to receive tasks
```

#### Binary Distribution Strategy

Rather than requiring Rust on every remote machine, nexus ships pre-built binaries:

| Strategy | When |
|----------|------|
| **Pre-built in ~/.hex/bin/** | Default — check `--build-hash` matches nexus version |
| **scp from nexus host** | If binary missing or version mismatch |
| **cargo build on remote** | Fallback if no pre-built binary for the arch |

The binary is ~15MB (release, stripped). SCP over LAN takes <1s.

#### Environment Variables Injected at Launch

```bash
HEX_NEXUS_URL=http://127.0.0.1:$AGENT_PORT  # tunneled nexus
HEX_AGENT_ID=$UUID                            # assigned by nexus
HEX_AGENT_TOKEN=$SESSION_TOKEN                # per-session auth
HEX_PROJECT_DIR=$PROJECT_DIR                  # working directory
```

## Consequences

### Positive
- **Zero exposed ports**: SSH tunnels mean remote machines need no firewall changes
- **Unified coordination**: SpacetimeDB provides real-time state sync without custom polling
- **Inference fleet**: Multiple GPU boxes contribute models to a shared inference pool
- **Graceful degradation**: If remote agent drops, tasks reassign automatically
- **Familiar UX**: SSH-based connectivity mirrors VS Code Remote SSH mental model

### Negative
- **SSH dependency**: Requires SSH key setup between machines (no zero-config for remote)
- **Latency**: SSH tunnel adds ~2-5ms per message vs direct WebSocket
- **SpacetimeDB coupling**: Remote coordination requires SpacetimeDB instance (can't use SQLite-only mode)
- **Complexity**: Three communication channels (SSH + WS + SpacetimeDB) to debug

### Risks
- **`russh` maturity**: Less battle-tested than OpenSSH — may hit edge cases with key formats
- **Tunnel stability**: Long-lived SSH tunnels can drop silently; keepalive + reconnect is critical
- **SpacetimeDB scaling**: Unknown behavior with 10+ concurrent agent subscriptions
- **Secret leakage**: If SSH tunnel is compromised, all forwarded secrets are exposed

### Mitigations
- Start with ssh-agent auth (most reliable), add direct key support later
- Implement aggressive keepalive (15s) + reconnect with exponential backoff (1s, 2s, 4s, 8s, max 60s)
- Load-test SpacetimeDB with simulated agent fleet before production use
- Rotate per-session nonces; log all secret access attempts

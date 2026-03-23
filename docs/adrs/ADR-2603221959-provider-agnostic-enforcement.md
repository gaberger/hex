# ADR-2603221959: Provider-Agnostic Enforcement via MCP Tool Guards

**Status:** Accepted
**Date:** 2026-03-22
**Drivers:** hex enforcement (swarm tracking, workplan gates, boundary validation) is currently tied to Claude Code's hook system (`.claude/settings.json`). Open models (Ollama, vLLM, Qwen, Llama) and alternative frontends (Continue, Cursor, custom harnesses) have no hook infrastructure — all enforcement is bypassed. hex must enforce architecture rules regardless of which LLM provider drives the agents.

## Context

hex's enforcement pipeline (ADR-2603221939) relies on Claude Code hooks:

```
PreToolUse(Agent)  → hex hook pre-agent   → blocks untracked agents
SubagentStart      → hex hook subagent-start → assigns HexFlo tasks
UserPromptSubmit   → hex hook route       → heartbeat, inbox, workplan check
PreToolUse(Edit)   → hex hook pre-edit    → boundary validation
```

These hooks are a **Claude Code-specific feature**. When hex agents run on open models via the inference broker (ADR-030), none of these hooks fire. The agent can:

- Edit files outside its workplan boundary — no `pre-edit` check
- Spawn sub-tasks without swarm tracking — no `pre-agent` gate
- Skip heartbeats — no `route` hook
- Make destructive changes — no `pre-bash` warning

### Current enforcement by provider

| Enforcement | Claude Code | MCP Client (any) | Direct REST | Open Model (no MCP) |
|-------------|-------------|-------------------|-------------|---------------------|
| Swarm tracking | Hook ✓ | None ✗ | None ✗ | None ✗ |
| Workplan gates | Hook ✓ | None ✗ | None ✗ | None ✗ |
| Boundary validation | Hook ✓ | None ✗ | None ✗ | None ✗ |
| Agent registration | Hook ✓ | None ✗ | REST ✓ | None ✗ |
| Heartbeat | Hook ✓ | None ✗ | REST ✓ | None ✗ |

The only provider-agnostic enforcement point is the **MCP tool layer** — `hex mcp` serves tools via stdio to any MCP-compatible client, and **hex-nexus REST API** serves the same operations to any HTTP client.

## Decision

### Move enforcement from client hooks to server-side tool guards

Enforcement logic moves to two layers that are provider-agnostic:

1. **MCP tool guards** — validation runs inside `hex mcp` tool handlers before executing operations
2. **Nexus API guards** — validation runs in hex-nexus REST handlers before mutating state

Both layers share the same enforcement logic (extracted into `hex-core` as a port).

### Architecture

```
┌──────────────────────────────────────────────────┐
│                    LLM Provider                   │
│  (Claude / Ollama / vLLM / Qwen / Custom)        │
└────────────┬─────────────────────┬───────────────┘
             │ MCP (stdio)         │ HTTP (REST)
             ▼                     ▼
┌────────────────────┐  ┌──────────────────────┐
│   hex mcp server   │  │   hex-nexus daemon   │
│  ┌──────────────┐  │  │  ┌──────────────┐    │
│  │ Tool Guards  │  │  │  │ API Guards   │    │
│  │ ─ workplan?  │  │  │  │ ─ agent reg? │    │
│  │ ─ swarm?     │  │  │  │ ─ boundary?  │    │
│  │ ─ boundary?  │  │  │  │ ─ ownership? │    │
│  └──────────────┘  │  │  └──────────────┘    │
│         │          │  │         │             │
│         ▼          │  │         ▼             │
│  Execute operation │  │  Execute operation    │
└────────────────────┘  └──────────────────────┘
```

### P1: Enforcement port in hex-core

Define an `IEnforcementPort` trait:

```rust
// hex-core/src/ports/enforcement.rs
pub struct EnforcementContext {
    pub agent_id: Option<String>,
    pub workplan_id: Option<String>,
    pub swarm_id: Option<String>,
    pub task_id: Option<String>,
    pub target_file: Option<String>,
    pub operation: String, // "edit", "write", "spawn_agent", "bash"
}

pub enum EnforcementResult {
    Allow,
    Warn(String),
    Block(String),
}

pub trait IEnforcementPort: Send + Sync {
    fn check(&self, ctx: &EnforcementContext) -> EnforcementResult;
}
```

### P2: MCP tool guards

Every MCP tool that mutates state checks enforcement before executing:

```rust
// In hex mcp tool handler
async fn handle_tool_call(name: &str, input: Value) -> Value {
    let ctx = EnforcementContext {
        agent_id: extract_agent_id(&input),
        workplan_id: extract_workplan_id(&input),
        task_id: extract_task_id(&input),
        target_file: input["path"].as_str().map(String::from),
        operation: name.to_string(),
    };

    match enforcer.check(&ctx) {
        EnforcementResult::Block(reason) => {
            return json!({ "error": reason, "blocked": true });
        }
        EnforcementResult::Warn(msg) => {
            // Include warning in tool output — LLM sees it regardless of provider
            eprintln!("[hex] WARNING: {}", msg);
        }
        EnforcementResult::Allow => {}
    }

    // Execute the actual tool...
}
```

### P3: Nexus API guards

hex-nexus REST endpoints check enforcement via middleware:

```rust
// axum middleware
async fn enforcement_middleware(
    State(state): State<SharedState>,
    request: Request,
    next: Next,
) -> Response {
    let ctx = build_enforcement_context(&request);
    match state.enforcer.check(&ctx) {
        EnforcementResult::Block(reason) => {
            return (StatusCode::FORBIDDEN, Json(json!({ "error": reason }))).into_response();
        }
        _ => next.run(request).await,
    }
}
```

### P4: Agent session via MCP

For providers without hook infrastructure, hex provides MCP tools for session lifecycle:

| MCP Tool | Replaces Hook | Purpose |
|----------|---------------|---------|
| `hex_session_start` | SessionStart hook | Register agent, load context |
| `hex_session_heartbeat` | UserPromptSubmit hook | Keep agent alive |
| `hex_workplan_activate` | route hook workplan check | Set active workplan |
| `hex_task_start` | SubagentStart hook | Assign task to agent |
| `hex_task_complete` | SubagentStop hook | Mark task done |

The LLM's system prompt instructs it to call `hex_session_start` at the beginning and `hex_session_heartbeat` periodically. This works with any MCP-compatible client.

### P5: Enforcement rules in SpacetimeDB

Move enforcement rules from `.hex/adr-rules.toml` (file-based, client-only) to a SpacetimeDB `enforcement_rule` table:

```
enforcement_rule {
    id: String,
    adr: String,
    operation: String,      // "edit", "spawn_agent", "bash", "*"
    condition: String,       // "requires_workplan", "requires_task", "boundary_check"
    severity: String,        // "block", "warn", "info"
    enabled: bool,
    project_id: Option<String>,  // null = global
}
```

Rules are synced from `.hex/adr-rules.toml` on startup (same pattern as config sync, ADR-044) but can also be managed via REST/MCP:

```bash
hex enforce list                    # Show all rules
hex enforce add --adr ADR-056 ...   # Add rule
hex enforce disable <rule-id>       # Disable without deleting
```

### P6: System prompt injection for open models

For models without MCP support, hex-agent injects enforcement instructions directly into the system prompt:

```
You are operating under hex architecture enforcement. Before editing files:
1. Call hex_workplan_activate with your workplan ID
2. Call hex_task_start with your task ID
3. Only edit files within your assigned hex layer boundary
Violations will be rejected by the hex-nexus API.
```

This is defense-in-depth — the server-side guards catch violations even if the model ignores prompt instructions.

## Consequences

**Positive:**
- Enforcement works with any LLM provider — Claude, Ollama, vLLM, Qwen, Llama, etc.
- Server-side guards are unforgeable — models can't bypass them regardless of prompt
- MCP tools provide the same lifecycle management as Claude Code hooks
- Rules stored in SpacetimeDB are consistent across all clients and sessions
- Client hooks (ADR-2603221939) remain as first-line defense for Claude Code — defense in depth

**Negative:**
- MCP tool overhead — each mutating call checks enforcement (~1ms)
- Open models need system prompt guidance — less reliable than hook enforcement
- Two enforcement paths to maintain (client hooks + server guards)
- Models without MCP support get weaker enforcement (prompt-only + API guards)

**Mitigations:**
- Enforcement check is in-memory (no network call) — sub-millisecond
- Client hooks and server guards share `IEnforcementPort` — single implementation
- System prompt injection is generated from the same rules as guards
- API guards are the ultimate backstop — even a jailbroken model can't bypass REST validation

## Implementation

| Phase | Description | Status |
|-------|------------|--------|
| P1 | `IEnforcementPort` trait in hex-core with `EnforcementContext` and rules engine | Done |
| P2 | MCP tool guards — check enforcement before executing mutating tools | Done |
| P3 | MCP lifecycle tools — session_start, heartbeat, workplan_activate | Done |
| P4 | Nexus API guards — axum middleware for REST endpoint enforcement | Done |
| P5 | Enforcement rules in SpacetimeDB — sync from .hex/adr-rules.toml + `hex enforce` CLI | Done |
| P6 | System prompt injection for non-MCP models — `hex enforce prompt` | Done |

## References

- ADR-2603221939: Mandatory Swarm Tracking (client-side enforcement via Claude Code hooks)
- ADR-030: Multi-Provider Inference Broker (model-agnostic inference routing)
- ADR-050: Hook-Enforced Agent Lifecycle Pipeline (Claude Code-specific)
- ADR-044: Config Sync (file → SpacetimeDB pattern)
- ADR-019: CLI-MCP Parity (every command has an MCP equivalent)

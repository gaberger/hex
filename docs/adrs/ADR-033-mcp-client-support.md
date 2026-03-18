# ADR-033: MCP Client Support for hex-agent

## Status

Proposed

## Context

hex-agent is a Rust binary that implements an autonomous AI agent using Anthropic's Messages API with native `tool_use`/`tool_result` protocol. It currently has 15 hardcoded built-in tools (read_file, write_file, bash, glob, grep, etc.) defined in `domain/tools.rs`.

Meanwhile, the hex TypeScript CLI exposes 30+ tools via MCP (Model Context Protocol) through `src/adapters/primary/mcp-adapter.ts`. Users may also have other MCP servers configured in their `.claude/settings.json` (databases, APIs, custom tools).

**The gap**: hex-agent cannot call MCP tools. When deployed as an autonomous agent (via hex-hub spawn or CLI), it's limited to its 15 built-in tools and cannot leverage the broader MCP ecosystem that Claude Code users expect.

### Why MCP Matters for hex-agent

1. **Tool parity with Claude Code**: Users expect hex-agent to have access to the same MCP tools as their Claude Code session (Sentry, GitHub, custom servers).
2. **HexFlo integration**: The hex MCP server provides `hex_hexflo_*` tools — hex-agent should be able to call these for swarm coordination.
3. **Extensibility**: Users should be able to add domain-specific tools (database queries, API calls, deployment) without modifying hex-agent source code.
4. **Multi-agent coordination**: When hex-hub spawns multiple agents, they should share access to the same MCP tool surface.

## Decision

Implement MCP client support in hex-agent using the stdio transport, following the hexagonal architecture pattern.

### Architecture

```
hex-agent/src/
  domain/
    mcp.rs              # MCP types: JsonRpcMessage, McpTool, McpToolResult
  ports/
    mcp_client.rs       # IMcpClientPort trait
  adapters/secondary/
    mcp_stdio_client.rs # Stdio transport: spawn process, JSON-RPC over stdin/stdout
    mcp_config.rs       # Load MCP server configs from .claude/settings.json
  usecases/
    mcp_discovery.rs    # Connect to servers, call tools/list, merge with builtins
```

### Port Interface

```rust
#[async_trait]
pub trait McpClientPort: Send + Sync {
    /// Connect to an MCP server and perform initialization handshake.
    async fn connect(&self, config: &McpServerConfig) -> Result<McpConnection, McpError>;

    /// Discover available tools from a connected server.
    async fn list_tools(&self, conn: &McpConnection) -> Result<Vec<McpToolDef>, McpError>;

    /// Call a tool on a connected server.
    async fn call_tool(
        &self,
        conn: &McpConnection,
        name: &str,
        arguments: serde_json::Value,
    ) -> Result<McpToolResult, McpError>;

    /// Disconnect from a server.
    async fn disconnect(&self, conn: McpConnection) -> Result<(), McpError>;
}
```

### Stdio Transport

The stdio transport spawns an MCP server process and communicates via JSON-RPC 2.0 over stdin/stdout:

```
hex-agent ──stdin──→ MCP server process
           ←stdout──
```

Protocol flow:
1. Spawn process with `command` and `args` from config
2. Send `initialize` request with client capabilities
3. Send `initialized` notification
4. Call `tools/list` to discover available tools
5. For each tool_use from Anthropic: send `tools/call` request
6. On shutdown: send `shutdown` notification, terminate process

### Configuration

MCP servers are loaded from `.claude/settings.json` (same format as Claude Code):

```json
{
  "mcpServers": {
    "hex": {
      "command": "hex",
      "args": ["mcp", "start"],
      "type": "stdio"
    },
    "sentry": {
      "command": "npx",
      "args": ["-y", "@sentry/mcp-server"],
      "type": "stdio"
    }
  }
}
```

hex-agent reads from:
1. Project-level: `.claude/settings.json` in the project directory
2. User-level: `~/.claude/settings.json` (merged, project takes precedence)
3. Hub-provided: MCP configs passed via `--mcp-config` CLI arg or hub spawn config

### Tool Dispatch Routing

When Anthropic responds with `tool_use`, the tool executor routes:

```rust
async fn execute(&self, call: &ToolCall) -> ToolResult {
    // 1. Check built-in tools first (fast path)
    if let Some(result) = self.try_builtin(&call.name, &call.input).await {
        return result;
    }

    // 2. Check MCP tools (by server prefix or full name)
    if let Some(result) = self.try_mcp_tool(&call.name, &call.input).await {
        return result;
    }

    // 3. Unknown tool
    ToolResult::error(format!("Unknown tool: {}", call.name))
}
```

MCP tool names use the Claude Code convention: `mcp__<server>__<tool>`.

### Tool Merging Strategy

At startup, hex-agent:
1. Loads built-in tools (15 tools)
2. Connects to each configured MCP server
3. Calls `tools/list` on each
4. Prefixes each tool name with `mcp__<server>__`
5. Merges all tools into the `tools` array sent to Anthropic API
6. Total tool list = builtins + all MCP tools

### Safety Constraints

- MCP server processes are spawned with restricted environment (no inherited secrets)
- Tool calls are logged with the server name for auditability
- Configurable tool allowlist/denylist per server
- Connection timeout: 10 seconds for initialization
- Tool call timeout: 120 seconds (configurable)
- Max concurrent MCP connections: 8

## Implementation Phases

| Phase | Layer | Description | Files |
|-------|-------|-------------|-------|
| 1 | Domain | MCP types (JsonRpc, McpTool, McpServerConfig) | `domain/mcp.rs` |
| 2 | Ports | `McpClientPort` trait | `ports/mcp_client.rs` |
| 3 | Secondary Adapter | Stdio transport (spawn, JSON-RPC, lifecycle) | `adapters/secondary/mcp_stdio_client.rs` |
| 4 | Secondary Adapter | Config loader (read .claude/settings.json) | `adapters/secondary/mcp_config.rs` |
| 5 | Usecase | Tool discovery (connect, list, merge) | `usecases/mcp_discovery.rs` |
| 6 | Secondary Adapter | Extend ToolExecutorAdapter with MCP routing | `adapters/secondary/tools.rs` |
| 7 | Composition Root | Wire MCP client into main.rs | `main.rs` |
| 8 | Tests | Integration tests with mock MCP server | `tests/mcp_client_test.rs` |

### Estimated LOC

- Domain types: ~150 LOC
- Port trait: ~50 LOC
- Stdio transport: ~400 LOC
- Config loader: ~100 LOC
- Discovery usecase: ~150 LOC
- Tool routing changes: ~50 LOC
- Composition root: ~30 LOC
- Tests: ~300 LOC
- **Total: ~1,230 LOC**

## Consequences

### Positive

- **Tool parity**: hex-agent gains access to all MCP tools available in Claude Code
- **Extensibility**: Users add tools via configuration, not code changes
- **HexFlo native access**: hex-agent can call hex MCP tools for swarm coordination
- **Multi-agent**: Hub-spawned agents share the same tool surface
- **Ecosystem**: Access to growing MCP server ecosystem (Sentry, GitHub, databases, etc.)

### Negative

- **Startup latency**: Connecting to N MCP servers adds ~N×1s to startup
- **Process management**: Must handle MCP server crashes, restarts, stderr
- **Complexity**: JSON-RPC protocol implementation, connection lifecycle management
- **Security surface**: Spawning external processes with tool access

### Neutral

- Built-in tools remain the fast path — no performance regression for existing tools
- MCP support is optional — hex-agent works fine without any MCP servers configured
- The stdio transport is the same mechanism Claude Code uses

## Related

- ADR-001: Hexagonal Architecture (ports & adapters pattern for MCP client)
- ADR-027: HexFlo (hex MCP tools that hex-agent should be able to call)
- ADR-019: CLI–MCP Parity (every CLI command has an MCP equivalent)
- [Model Context Protocol Specification](https://modelcontextprotocol.io/docs)

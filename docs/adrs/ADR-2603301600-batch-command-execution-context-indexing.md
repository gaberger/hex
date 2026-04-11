# ADR-2603301600: Batch Command Execution with Context Indexing

**Status:** Accepted
**Date:** 2026-03-30
**Drivers:** Agent context windows flood with raw command output (cargo build, cargo test, hex analyze), consuming tokens irreversibly and degrading reasoning quality
**Supersedes:** N/A

<!-- ID format: YYMMDDHHMM — use your local time. Example: 2603221500 = 2026-03-22 15:00 -->

## Context

When hex agents execute shell commands (build, test, analysis), the full output is returned directly into the Claude context window. A single `cargo build` error trace can be 200–500 lines. A `hex analyze .` report on a large project can exceed 1,000 lines. This has several consequences:

- **Token burn**: Raw output consumes context that can't be reclaimed
- **Reasoning degradation**: Signal-to-noise drops as irrelevant lines crowd out the task context
- **Cascading eviction**: The `ContextManagerAdapter` evicts older messages (including task state) to make room, losing important earlier context

The existing `ContextManagerAdapter` only manages conversation history packing — it has no mechanism to intercept or redirect command output before it enters the context window.

The `plugin:context-mode` MCP plugin (external tool) demonstrates the correct pattern: `ctx_batch_execute` runs commands, indexes the output in a sandbox, and returns only query-matched excerpts. This ADR formalises that pattern as a **first-class hex port and adapter**, so it works natively within hex-agent without requiring the external plugin.

### Alternatives considered

1. **Keep relying on the external context-mode plugin** — requires plugin to be installed in every hex-agent environment; not portable, not testable, not part of hex's own architecture
2. **Truncate command output at the MCP tool layer** — blunt, loses important tail errors; no semantic filtering
3. **Summarize output with a Haiku preflight** — adds LLM latency + cost to every command; overkill for structured output like compiler errors
4. **Stream output to a file and read selectively** — works but no indexing or search; still pollutes context on read

## Decision

We will introduce a `IBatchExecutionPort` in hex-agent's ports layer and a `CommandSessionAdapter` as its secondary adapter implementation. This provides sandboxed command execution with in-memory output indexing and substring/pattern search — keeping raw output out of the agent's context window.

### Port interface (`hex-agent/src/ports/command_session.rs`)

```rust
#[async_trait]
pub trait IBatchExecutionPort: Send + Sync {
    /// Run commands sequentially, index all output. Returns a session ID.
    async fn batch_execute(
        &self,
        commands: Vec<String>,
        working_dir: &Path,
    ) -> Result<BatchSession, CommandSessionError>;

    /// Search indexed output for matching lines.
    async fn search(
        &self,
        session_id: &str,
        queries: Vec<String>,
    ) -> Result<Vec<SearchResult>, CommandSessionError>;

    /// Discard a session's indexed output (free memory).
    async fn drop_session(&self, session_id: &str);
}

pub struct BatchSession {
    pub session_id: String,
    pub commands_run: usize,
    pub total_lines: usize,
    pub exit_codes: Vec<i32>,
}

pub struct SearchResult {
    pub command: String,
    pub line_number: usize,
    pub line: String,
    pub score: f32,  // 1.0 = exact match, <1.0 = fuzzy
}
```

### Secondary adapter (`hex-agent/src/adapters/secondary/command_session.rs`)

- Executes each command via `tokio::process::Command` with a configurable timeout (default 60s)
- Stores output as `HashMap<session_id, Vec<IndexedLine>>` where `IndexedLine = (command, line_no, text)`
- Search: exact substring match first (score=1.0), then case-insensitive (score=0.9), then token overlap (score=0.5–0.8)
- Sessions are TTL-evicted after 10 minutes of inactivity
- Max 500MB indexed output per agent instance (configurable via env `HEX_CMD_SESSION_MAX_MB`)

### MCP tool exposure (`hex mcp`)

Two new MCP tools served by the hex-cli MCP server:

| Tool | Description |
|------|-------------|
| `hex_batch_execute` | Run commands and index output; returns session_id + summary stats |
| `hex_batch_search` | Search a session's indexed output; returns matched lines with context |

### hex-nexus REST endpoint (optional, for dashboard visibility)

`POST /api/command-sessions` — proxies to the local agent's `IBatchExecutionPort`. Allows the dashboard to show active command sessions and their output stats. Sessions are agent-local; nexus does not store output centrally.

### Scope boundaries

- **In scope**: hex-agent secondary adapter, port interface, MCP tools, nexus proxy route
- **Out of scope**: SpacetimeDB persistence of session data (transient by design), cross-agent session sharing, output streaming (batch-then-search only)
- **Does not replace**: `ContextManagerAdapter` (still needed for conversation history packing)

## Consequences

**Positive:**
- Agent context windows stay clean — only query-matched excerpts enter context
- Consistent pattern across all agents (not dependent on external plugin)
- Testable in isolation (port interface enables mock injection per ADR-014)
- Composable with existing `ContextManagerAdapter` — both can operate independently

**Negative:**
- In-memory output store adds resident memory pressure to hex-agent processes
- Two-step interaction pattern (execute → search) is slightly more complex than direct output
- Search quality is basic (substring/token overlap, not semantic embeddings)

**Mitigations:**
- TTL eviction + size cap prevents unbounded memory growth
- Search quality is sufficient for structured output (compiler errors, test results, analysis reports) which are keyword-rich; semantic search not needed
- MCP tool design hides the two-step pattern from agents — `hex_batch_execute` returns a session handle, `hex_batch_search` is the natural follow-up

## Implementation

| Phase | Description | Status |
|-------|-------------|--------|
| P1 | Define `IBatchExecutionPort` trait + error types in `hex-agent/src/ports/` | Pending |
| P2 | Implement `CommandSessionAdapter` with subprocess execution + in-memory index | Pending |
| P3 | Wire adapter into `hex-agent` composition root | Pending |
| P4 | Expose `hex_batch_execute` + `hex_batch_search` MCP tools in `hex-cli/src/commands/mcp.rs` | Pending |
| P5 | Add nexus proxy route `POST /api/command-sessions` | Pending |
| P6 | Unit tests: port mock, adapter integration test with real subprocess | Pending |

## References

- `hex-agent/src/adapters/secondary/context_manager.rs` — existing context packing adapter
- `hex-agent/src/adapters/secondary/task_executor.rs` — existing task execution pattern to follow
- ADR-014: Dependency injection via Deps pattern (no `mock.module()`)
- ADR-027: HexFlo native Rust coordination
- `plugin:context-mode` external MCP plugin — the inspiration for this pattern

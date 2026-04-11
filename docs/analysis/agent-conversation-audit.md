# hex-agent Conversation Loop & Tool Execution Audit

**Date**: 2026-03-18
**Scope**: `hex-agent/src/` — conversation loop, tool executor, context management, Anthropic adapter, hub-managed mode
**Status**: Research only — no code changes

---

## 1. Conversation Loop (`usecases/conversation.rs`)

### Full Turn Trace

```
User input
  → RL engine queried for context strategy (Aggressive/Balanced/Conservative)
  → Token budget partitions adjusted by RL multipliers
  → User message pushed to ConversationState
  → LOOP begins:
      → ContextManagerPort.pack() trims history to fit budget
      → AnthropicPort.send_message() called (non-streaming, full response)
      → TokenUpdate event emitted
      → Response content blocks iterated:
          - Text → TextChunk event emitted, block saved
          - ToolUse → ToolCallStart event, tool executed via ToolExecutorPort,
                       ToolCallResult event, ToolResult block saved
      → Assistant message pushed to state
      → If tool_use blocks exist:
          - Tool results pushed as User message (role: User, content: [ToolResult...])
          - tool_rounds incremented; if >= max_tool_rounds (25), break with error event
          - CONTINUE loop (model processes tool results)
      → If no tool_use → TurnComplete event, BREAK
  → RL reward reported (token efficiency signal, tool round penalty)
```

### Key Design Decisions

- **Non-streaming mode used in the loop**: `send_message()` collects the full response before processing. The `stream_message()` port exists but is unused — the conversation loop does not stream.
- **Tool results are User messages**: Following Anthropic's API contract, tool results are sent as `role: user` with `ContentBlock::ToolResult` blocks.
- **Max 25 tool rounds per turn**: Hard limit prevents runaway tool loops. Emits an Error event but does NOT return an error — the turn "succeeds" with partial results.
- **RL integration is best-effort**: `select_action` and `report_reward` failures are silently ignored (`.ok()`). The NoopRlAdapter is used when no hub is connected.

---

## 2. Tool Execution (`adapters/secondary/tools.rs`)

### Tool Inventory

| Tool | Input Validation | Path Safety | Error Handling |
|------|-----------------|-------------|----------------|
| `read_file` | Requires `path` | `safe_path()` | Returns `is_error: true` with message |
| `write_file` | Requires `path`, `content` | `safe_path()` | Creates parent dirs; error on write fail |
| `edit_file` | Requires `path`, `old_string`, `new_string` | `safe_path()` | Rejects if old_string not found or matches >1 time |
| `glob_files` | Requires `pattern` | Joins to working_dir | Caps at 200 results |
| `grep_search` | Requires `pattern` | Runs `rg` via Command | Returns "No matches" on empty |
| `bash` | Requires `command` | working_dir pinned | Timeout (default 120s), captures stdout+stderr |
| `list_directory` | Requires `path` | `safe_path()` | Sorted output |
| `worktree_create` | Requires `branch` | Path derived from repo name | Reports git stderr on failure |
| `worktree_status` | None | N/A | Parses porcelain output |
| `worktree_merge` | Requires `branch` | Verifies worktree exists | Pre-merge verification command support |
| `worktree_remove` | Requires `branch` | Finds worktree by branch | Optional branch deletion |

### Security Analysis

**Path Traversal Protection (`safe_path()`)**:
- Canonicalizes paths (resolving `..` and symlinks)
- For new files, canonicalizes the parent directory
- Verifies the resolved path starts with the canonicalized working directory
- Rejects absolute paths outside the project root
- **Gap**: Symlinks created WITHIN the project that point outside are resolved by `canonicalize()` and would be rejected. This is correct.

**Command Injection**:
- `bash` tool: Uses `/bin/sh -c <command>` via `tokio::process::Command`. The command string IS passed through a shell, so the LLM can execute arbitrary shell commands. This is by design (same as Claude Code's bash tool) but represents the primary attack surface.
- `grep_search`: Uses `Command::new("rg")` with separate args — no shell injection possible.
- `worktree_*` tools: Use `Command::new("git")` with explicit args — safe.
- `worktree_merge` verification: Uses `Command::new("sh")` with `-c` — the verify_command comes from tool input (LLM-controlled), same risk as bash tool.

**Missing Protections**:
- No file size limits on `read_file` or `write_file` — a multi-GB file could blow memory
- No output size limits on `bash` tool — stdout/stderr fully captured
- No allowlist/blocklist for bash commands (e.g., `rm -rf /`)
- `glob_files` does NOT use `safe_path()` — the pattern is joined to working_dir but the glob library may follow symlinks outside the project

### Error Handling Pattern

All tools return `ToolResult { is_error: true, content: "Error in <tool>: <msg>" }` on failure. Errors are NOT propagated as Rust `Result::Err` — the conversation loop always gets a `ToolResult` and feeds it back to the model. This is correct behavior: the LLM should see tool errors and decide how to recover.

---

## 3. Context Window Management

### Architecture

```
TokenBudget (domain/tokens.rs)
  ├── max_context: 200,000 (default, configurable via --max-context)
  ├── response_reserve: 15% (30,000 tokens)
  └── partitions (of remaining 170,000):
      ├── system_fraction: 15% (25,500 tokens)
      ├── history_fraction: 40% (68,000 tokens)
      └── tool_fraction: 30% (51,000 tokens)
          (remaining 15% is unused headroom)

ContextManagerAdapter (adapters/secondary/context_manager.rs)
  ├── Token counting: chars / 4 heuristic (no tiktoken)
  ├── Packing strategy: reverse chronological — newest messages first
  ├── Eviction: oldest messages dropped when history_budget exceeded
  └── Summarization: extractive (first line of each evicted message)
```

### RL-Adjusted Budgets

The RL engine can modify partition fractions via multipliers:
- **Aggressive**: history x1.3, tools x1.2 (favors more context)
- **Conservative**: history x0.7, tools x0.8 (favors saving tokens)
- These multipliers can cause fractions to exceed 1.0 total — **no normalization is applied**

### Eviction Algorithm

1. Walk messages from newest to oldest
2. Include each message if its token count fits remaining budget
3. **Stop on first message that doesn't fit** — does NOT skip large messages to include smaller older ones
4. Build packed list preserving original order

### Gaps in Context Management

- **No summarization during eviction**: The `summarize()` method exists on the port but is NEVER called by the conversation loop or the context manager's `pack()` method. Evicted messages are simply dropped.
- **Tool result partition unused**: The `tool_fraction` budget is defined but never separately enforced. All messages (including tool results) compete in the single `history_budget` pool.
- **char/4 heuristic is inaccurate**: Over-counts for code (many short tokens), under-counts for natural language. Can lead to premature eviction or context overflow.
- **No pinning**: The comment says "pinned tool results are preserved" but the implementation does simple newest-first with no pinning logic.
- **Greedy eviction**: The break-on-first-overflow means a single large tool result near the beginning can cause all older messages to be evicted even if many smaller ones would fit.

---

## 4. Event Streaming

### Event Types

| Event | CLI Handling | Hub Forwarding |
|-------|-------------|----------------|
| `TextChunk(text)` | `print!("{}", text)` to stdout | `HubMessage::StreamChunk` |
| `ToolCallStart { name, input }` | Truncated to 80 chars on stderr | `HubMessage::ToolCall` |
| `ToolCallResult { name, content, is_error }` | Success/failure indicator on stderr | `HubMessage::ToolResultMsg` |
| `TokenUpdate(usage)` | `tracing::debug` only | `HubMessage::TokenUpdate` |
| `TurnComplete { stop_reason }` | Newline + max_tokens warning | **NOT forwarded** |
| `ContextReset { summary }` | Printed on stderr | **NOT forwarded** |
| `Error(msg)` | Printed on stderr | **NOT forwarded** |

### Hub-Managed Mode Event Gaps

The hub forwarder in `main.rs` (lines 341-373) uses a match with a catch-all `_ => None` that silently drops:
- `TurnComplete` — the hub never knows when a turn is done
- `ContextReset` — the hub never knows about context resets
- `Error` — tool-round-limit errors are lost

The hub DOES receive `AgentStatus { status: "error" }` when `process_message()` returns `Err`, but the tool-round-limit error emits an event and then returns `Ok(())` — so the hub never sees it.

---

## 5. Error Handling

### Anthropic API Errors

| Error Type | Adapter Behavior | Conversation Loop Behavior |
|-----------|-----------------|---------------------------|
| HTTP transport failure | `AnthropicError::Http(msg)` | Mapped to `ConversationError::ApiError`, returned to caller |
| 429 Rate Limited | `AnthropicError::RateLimited { retry_after_ms }` | **No retry logic** — mapped to ApiError, returned immediately |
| 4xx/5xx | `AnthropicError::Api { status, message }` | Mapped to ApiError, returned to caller |
| JSON parse failure | `AnthropicError::Deserialize(msg)` | Mapped to ApiError, returned to caller |
| Context overflow | `AnthropicError::ContextOverflow` | This error type exists but is NEVER produced by the adapter |

**Critical gap**: No retry logic for rate limits. The `retry_after_ms` is parsed from the response but never used. The error immediately propagates to the CLI/hub, ending the turn.

### Tool Execution Errors

Tool errors never crash the conversation loop. They produce `ToolResult { is_error: true }` which is fed back to the model as a tool_result content block. The model then decides whether to retry, try a different approach, or report the error.

### Context Overflow

If `pack()` fails with `SystemPromptTooLarge`, the error maps to `ConversationError::ContextError` and the turn fails. There is no fallback (e.g., truncating the system prompt).

If the API returns a context-overflow error (e.g., request too large for the model), it surfaces as a generic `ApiError`. The loop does not detect this and retry with a smaller context.

---

## 6. Production Gaps

### Critical

1. **No rate-limit retry**: 429s kill the turn immediately. Need exponential backoff with jitter.
2. **No streaming in conversation loop**: `send_message()` blocks until full response arrives. For long tool-use chains, the user sees no output until the entire API call completes. The `stream_message()` port exists but is unused.
3. **Tool-round-limit error is silent to hub**: The tool round limit (25) emits an event but returns `Ok(())`. Hub operators have no visibility into this failure mode.
4. **No output size limits**: bash tool stdout, file reads, and grep results have no byte caps. A single `cat /dev/urandom | head -c 10000000` could OOM the process or blow the context window.

### Important

5. **Token counting is approximate**: char/4 heuristic diverges significantly from actual tiktoken counts. This causes context packing to be unreliable — either wasting 20-30% of available context or occasionally exceeding the real limit (causing API-side 400 errors).
6. **Summarization is dead code**: `ContextManagerPort::summarize()` is implemented but never called. Evicted messages are silently dropped with no summary injected.
7. **RL budget multipliers are unclamped**: Aggressive strategy can push `history_fraction * 1.3 = 0.52` and `tool_fraction * 1.2 = 0.36`, totaling 1.03 (>1.0 of available budget). With system_fraction at 0.15, total is 1.03 — the fractions exceed 100%.
8. **`/plan` command leaks memory**: `Box::leak()` on line 82 of `cli.rs` leaks a string allocation every time `/plan <args>` is used. Over a long session, this grows unbounded.
9. **glob_files bypasses safe_path()**: The glob pattern is joined to working_dir but not validated. Glob patterns with `../` or absolute components could match files outside the project.
10. **No conversation persistence**: If the process crashes, all conversation state is lost. No checkpoint/resume mechanism beyond `reset_context()`.

### Nice-to-Have

11. **No tool concurrency**: Tools are executed sequentially even when multiple tool_use blocks are returned in a single response. Parallel execution would reduce latency.
12. **No tool timeout per-tool**: Only bash has a timeout (120s default). read_file on a FUSE mount or NFS could hang indefinitely.
13. **Hub event forwarding is lossy**: TurnComplete, ContextReset, and Error events are dropped in hub mode.
14. **ConversationState.new() inconsistency**: In `main.rs`, `ConversationState::new(system_prompt)` is called but `ConversationState::new()` takes a `conversation_id: String`, not a system prompt. The system_prompt is set separately. The code in main.rs line 313 passes `system_prompt.clone()` as the conversation_id — this is a **bug**: the conversation_id will be the full system prompt text instead of a UUID.

---

## 7. Architecture Summary

```
                    ┌─────────────────┐
                    │   CLI Adapter    │  (primary)
                    │   Hub-Managed    │  (primary)
                    └────────┬────────┘
                             │
                    ┌────────▼────────┐
                    │ ConversationLoop │  (use case)
                    │   ┌──────────┐  │
                    │   │ RL Query │  │
                    │   └──────────┘  │
                    └─┬──────┬──────┬─┘
                      │      │      │
           ┌──────────▼┐  ┌──▼───┐  ┌▼──────────┐
           │ Anthropic  │  │Context│  │   Tool    │  (secondary adapters)
           │  Adapter   │  │Manager│  │ Executor  │
           └────────────┘  └──────┘  └───────────┘
```

The hexagonal architecture is clean: the conversation loop depends only on ports. The composition root in `main.rs` wires adapters. Hub-managed vs CLI mode is a primary adapter swap. RL and SpacetimeDB adapters are conditionally composed based on `--hub-url` availability.

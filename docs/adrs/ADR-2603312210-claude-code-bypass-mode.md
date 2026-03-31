# ADR-2603312210: Claude Code Bypass Mode for hex-agent

**Status:** Proposed
**Date:** 2026-03-31
**Drivers:** `hex_plan_execute` fails when the Anthropic API key in the secrets vault has no credits, even when invoked from within an active Claude Code session that already has valid credentials.

## Context

When `hex_plan_execute` runs, it spawns `hex-agent` as a child process. `hex-agent` initialises its own `AnthropicAdapter`, runs a preflight API check, and fails immediately if the vault key has zero credits — even though the parent Claude Code session has full access.

Claude Code sets two reliable environment variables in every process it spawns:

| Variable | Value |
|---|---|
| `CLAUDECODE` | `1` |
| `CLAUDE_CODE_ENTRYPOINT` | `cli` |

When these are present, `hex-agent` is running inside a Claude Code session and can use `claude -p <prompt>` (non-interactive mode) to execute tasks through the session's own credentials rather than making direct API calls.

### Forces

- `hex-agent` direct API path requires a funded key in the secrets vault
- Claude Code sessions already have working credentials — no duplication needed
- The bypass must be transparent: same task input, same output contract
- Must not break direct invocation (outside Claude Code)

### Alternatives Considered

1. **Always require vault credits** — forces operators to maintain two separate funded accounts
2. **Pass session token via env** — Claude Code session tokens are not exportable
3. **MCP passthrough** — complex; requires parent session to expose a local MCP server
4. **`claude -p` subprocess** (chosen) — uses `claude` CLI already in PATH, inherits session credentials, zero new dependencies

## Decision

Add a **Claude Code bypass mode** to `hex-agent`:

### 1. Detection

```rust
fn is_claude_code_session() -> bool {
    std::env::var("CLAUDECODE").as_deref() == Ok("1")
        || std::env::var("CLAUDE_CODE_ENTRYPOINT").is_ok()
}
```

### 2. ClaudeCodeInferenceAdapter

New secondary adapter: `hex-agent/src/adapters/secondary/claude_code_inference.rs`

- Implements the same `InferenceAdapter` trait as `AnthropicAdapter`
- Spawns `claude -p "<prompt>"` as a subprocess
- Captures stdout as the response
- Passes `--allowedTools` and `--max-turns` flags as appropriate
- Returns `Err` if `claude` binary is not in PATH (fall through to direct adapter)

### 3. Startup selection

In `hex-agent/src/main.rs`, before initialising `AnthropicAdapter`:

```rust
let inference = if is_claude_code_session() && which::which("claude").is_ok() {
    tracing::info!("Claude Code session detected — using bypass mode");
    Box::new(ClaudeCodeInferenceAdapter::new()) as Box<dyn InferenceAdapter>
} else {
    Box::new(AnthropicAdapter::new(api_key, model)) as Box<dyn InferenceAdapter>
};
```

### 4. Skip preflight in bypass mode

Preflight calls the Anthropic API directly. Skip it when in bypass mode — the `claude` CLI handles its own auth.

### 5. Opt-out flag

`--no-claude-code-bypass` disables auto-detection for testing direct API path.

## Consequences

**Positive:**
- `hex_plan_execute` works inside Claude Code sessions with no vault credits required
- No new dependencies (uses `claude` CLI already on PATH)
- Transparent to `workplan_executor` — same spawn contract

**Negative:**
- `claude -p` starts a new session per task (no shared context with parent)
- Output parsing depends on `claude` CLI stdout format stability
- Adds a subprocess hop: hex-nexus → hex-agent → claude → Anthropic

**Mitigations:**
- Each workplan task is self-contained by design — no shared context needed
- `claude -p` output format is stable (plain text response)
- Fall-through: if `claude` not in PATH, use direct adapter as before

## Implementation

| Phase | Task |
|---|---|
| P1 | `is_claude_code_session()` detection fn + unit tests |
| P2 | `ClaudeCodeInferenceAdapter` — spawn `claude -p`, capture output |
| P3 | Wire into `main.rs` startup adapter selection |
| P4 | Skip preflight when bypass active |
| P5 | `--no-claude-code-bypass` flag |
| P6 | Integration test: verify task runs end-to-end via bypass |

## References

- `CLAUDECODE=1` env var — set by Claude Code CLI in all child processes
- `claude -p` — Claude Code non-interactive mode (print response and exit)
- ADR-2603312100: Context Engineering for hex-agent
- `hex-nexus/src/orchestration/agent_manager.rs` — spawn logic

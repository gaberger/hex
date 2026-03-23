# ADR-029: Haiku Preflight Checks & Automatic Context Compaction

**Status:** Accepted
**Date:** 2026-03-18
**Deciders:** Gary
**Relates to:** ADR-028 (API Optimization Layer)

## Context

Analysis of Claude Code's internal architecture (Kirshatrov, 2025) reveals two patterns that hex-agent does not yet implement:

1. **Startup quota check** — On session start, Claude Code fires a minimal Haiku 3.5 request to verify the API key is valid and the account has sufficient token quota. This fails fast (~200ms, ~50 tokens) instead of building a full context window (~15k tokens) and then discovering the key is invalid or rate-limited.

2. **Topic change detection ("check-new-topic")** — On each user input, Claude Code sends a lightweight Haiku classification prompt: "Is this a new conversation topic or a continuation?" If new topic, it triggers context compaction (summarize + clear history) before processing. This prevents context window pollution from unrelated prior turns.

3. **Automatic context compaction** — Claude Code triggers compaction when context usage exceeds a threshold, not just on manual request. hex-agent has `ConversationPort::reset_context()` but it's only called explicitly by the user or by `hex plan`.

These are cheap (~50 tokens each via Haiku) but architecturally significant because they introduce **multi-model orchestration within a single turn**: a cheap classifier model gates whether the expensive reasoning model runs and with what context.

## Decision

### 1. Preflight Port

Add a new `PreflightPort` to the ports layer:

```rust
#[async_trait]
pub trait PreflightPort: Send + Sync {
    /// Verify API connectivity and quota before the first turn.
    /// Returns Ok(()) if ready, Err with a user-facing message if not.
    async fn check_quota(&self) -> Result<(), PreflightError>;

    /// Classify whether user input represents a new topic.
    /// Returns true if context should be compacted before processing.
    async fn is_new_topic(
        &self,
        recent_context: &str,
        new_input: &str,
    ) -> Result<bool, PreflightError>;
}
```

### 2. Haiku Preflight Adapter

Implement `PreflightPort` using Haiku (cheapest model) with minimal prompts:

- **Quota check**: Send `{"messages": [{"role": "user", "content": "ping"}], "max_tokens": 1}` to Haiku. If it returns any response, quota is available. If 401/403/429, report the specific issue.
- **Topic detection**: Send a structured prompt with the last assistant message + new user input, ask Haiku to classify as `CONTINUE` or `NEW_TOPIC`. Parse the single-token response.

### 3. Auto-Compaction Policy

Integrate into `ConversationLoop.run_turn()`:

```
1. Before processing user input:
   a. If turn_count > 0: call preflight.is_new_topic(last_context, input)
   b. If new topic: call self.reset_context() automatically
   c. Log: "Topic change detected — compacting context"

2. After context packing:
   a. If packed.utilization > 0.85: trigger compaction
   b. Summarize conversation so far into checkpoint
   c. Clear history, inject summary as system context
```

### 4. CLI Integration

```
--no-preflight         Disable startup quota check and topic detection
--compact-threshold 85 Context utilization % that triggers auto-compaction (default: 85)
```

## Architecture Impact

This adds a new secondary adapter that calls the Anthropic API with a **different model** than the main conversation. The composition root wires it separately:

```rust
// Preflight uses Haiku regardless of the main model
let preflight_anthropic = Arc::new(AnthropicAdapter::new(api_key.clone(), "claude-haiku-4-5-20251001".into()));
let preflight: Arc<dyn PreflightPort> = Arc::new(HaikuPreflightAdapter::new(preflight_anthropic));

// Main conversation uses configured model (Sonnet/Opus)
let conversation = ConversationLoop::new(anthropic, ..., preflight);
```

This follows hex architecture: the `PreflightPort` is a port, the `HaikuPreflightAdapter` is a secondary adapter, and the `ConversationLoop` usecase composes them. The preflight adapter imports only from ports, never from other adapters.

## Cost Analysis

| Operation | Model | Tokens | Cost (per call) | Frequency |
|-----------|-------|--------|-----------------|-----------|
| Quota check | Haiku | ~50 in, ~1 out | ~$0.000013 | Once per session |
| Topic detection | Haiku | ~200 in, ~1 out | ~$0.00005 | Once per user turn |
| Main reasoning | Sonnet/Opus | ~20k in, ~2k out | ~$0.07-0.45 | Once per user turn |

Preflight overhead is <0.1% of total cost. The savings from avoiding unnecessary context pollution far outweigh the classification cost.

## Consequences

### Positive
- **Fail fast** — Invalid API keys or exhausted quotas are caught in <500ms instead of after building full context
- **Cleaner context** — Topic changes don't pollute the reasoning model's context with irrelevant history
- **Automatic compaction** — Users don't need to manually trigger `/compact` when context fills up
- **Cost savings** — Compacting early avoids sending bloated context on every subsequent turn

### Negative
- **Extra API call per turn** — ~200 tokens to Haiku for topic detection (negligible cost)
- **False positives** — Haiku may misclassify a related follow-up as a new topic, causing premature compaction
- **Latency** — ~200ms added per turn for the Haiku classification call (can be parallelized with context packing)

### Mitigations
- Topic detection prompt should be tuned to err on the side of `CONTINUE` (high precision, lower recall)
- Compaction preserves a summary checkpoint, so information is not lost — just compressed
- `--no-preflight` flag for latency-sensitive batch workloads

## Implementation Plan

1. Add `PreflightPort` to `src/ports/preflight.rs`
2. Add `HaikuPreflightAdapter` to `src/adapters/secondary/haiku_preflight.rs`
3. Modify `ConversationLoop` to accept `Option<Arc<dyn PreflightPort>>`
4. Add startup quota check in `main.rs` before entering the conversation loop
5. Add topic detection call at the start of `run_turn()`
6. Add auto-compaction threshold check after context packing

## References
- [Kirshatrov: Claude Code Internals](https://kirshatrov.com/posts/claude-code-internals)
- ADR-028: API Optimization Layer (prompt caching, rate limiting infrastructure this builds on)

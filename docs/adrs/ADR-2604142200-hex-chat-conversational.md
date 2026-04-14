# ADR-2604142200: Hex Chat — Conversational Brain Interface

**Status:** Proposed
**Date:** 2026-04-14
**Drivers:** `hex hey` is one-shot (classify → execute → exit). Users want a persistent conversation where brain maintains context, remembers prior messages, and reports proactively. An AIOS should talk, not just listen.

## Context

Today:
- `hex hey <text>` — single intent, runs once, exits
- `hex brain daemon` — runs silently in background, no dialog
- `hex inbox` — one-way notifications from brain to operator

Missing: **two-way persistent dialog** where the operator asks follow-up questions and brain has memory of the conversation + current project state.

## Decision

### 1. `hex chat` — already exists, extend it

hex already has `hex chat` (TUI + stdout modes). Extend it to:
- Auto-include brain state in context (queue, validate results, recent events)
- Answer "what are you doing?" / "status" with rich context
- Tool-use: brain can call `hex hey` internally to execute intents mid-conversation
- Stream brain_tick events inline so conversation shows "while we were talking, X completed"

### 2. Context bundle injected per message

Each user message is augmented with:
```
[brain context]
  daemon: running PID 20087, interval 10s
  queue: 2 pending (dashboard workplan, FIXME wp-hex-fs)
  last validate: 5 issues (13 unwired modules, 2 MCP gaps, 1 stale worktree)
  recent: committed a1ab082a native-fs ADR, 73f49215 dashboard workplan
[end context]
```

LLM sees this invisibly, user sees natural answers.

### 3. Conversational status intents

Add to `classify_intent` — when text matches "status", "what are you doing", "how's it going" AND chat session is active: respond conversationally, not with raw command output.

### 4. Interactive brain daemon watch

`hex chat --with-daemon` — subscribes to brain_tick stream. When brain completes a task mid-conversation, injects inline: "↪ just finished: cleanup of 3 merged worktrees."

### 5. Memory persistence

Conversation history stored in HexFlo memory: `chat-session:{id}:{timestamp}`. New sessions can `hex chat --resume` to continue prior conversation.

## Consequences

**Positive:**
- hex becomes genuinely interactive — not just a batch dispatcher
- User gets proactive updates ("I just finished X")
- Follow-up questions work (context preserved)
- Friendly personality, not just command output

**Negative:**
- More LLM calls = more compute
- Context bundle adds token overhead
- Streaming is complex

**Mitigations:**
- Context bundle capped at 500 tokens
- Use local qwen3:4b for status chat (free)
- Frontier models only for complex multi-step reasoning

## Implementation

| Phase | Description | Status |
|-------|------------|--------|
| P1 | Context bundle injector (queue, validate, recent commits) | Pending |
| P2 | Status intent in classify_intent + chat-session-aware response | Pending |
| P3 | brain_tick event stream integration into chat | Pending |
| P4 | Chat session persistence to HexFlo memory | Pending |
| P5 | `hex chat --resume` for continuing prior sessions | Pending |

## References

- ADR-2604140000: Hey Hex (one-shot classifier)
- ADR-2604132300: Brain Daemon Loop
- ADR-2604141100: Brain Updates to Operator
- Existing `hex chat` command (ADR-2603231800 opencode integration)

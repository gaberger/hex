# ADR-2604140000: Hey Hex — Natural Language Task Enqueue

**Status:** Proposed
**Date:** 2026-04-14
**Drivers:** `hex brain enqueue hex-command -- "--version"` is friction. Users want to say "hey brain, do X" and have brain figure it out. An AIOS should have a voice-assistant-style natural-language surface, not just structured CLI.

## Context

Today enqueuing requires:
- Knowing the task kind (`hex-command`, `workplan`, `shell`)
- Quoting args correctly (dealing with clap `--` escaping)
- Remembering the payload format

Natural language is how humans think. "Hey Siri, set a timer for 5 minutes." is better than "siri timer create --duration 5m --unit minutes --beep true".

Brain should accept free-form intent and classify it into a task.

## Decision

### 1. New top-level command: `hey`

```bash
hex hey calibrate all local inference providers
hex hey run the brain daemon workplan
hex hey what's broken?
hex hey clean up old worktrees
hex hey rebuild and restart nexus
```

Single command. Everything after `hey` is natural language.

### 2. Intent classification (deterministic first, inference as fallback)

**Tier 1 — keyword match (no LLM):**

```rust
fn classify_intent(text: &str) -> TaskIntent {
    let t = text.to_lowercase();
    if t.contains("calibrate") && t.contains("inference") {
        return TaskIntent::HexCommand("inference setup".into());
    }
    if t.contains("rebuild") {
        return TaskIntent::Shell("cargo build -p hex-cli -p hex-nexus --release".into());
    }
    if t.contains("reconcile") && t.contains("workplan") {
        return TaskIntent::HexCommand("plan reconcile --all --update".into());
    }
    if t.contains("clean") && t.contains("worktree") {
        return TaskIntent::HexCommand("worktree cleanup --force".into());
    }
    if t.contains("what") && (t.contains("broken") || t.contains("wrong")) {
        return TaskIntent::HexCommand("brain validate".into());
    }
    // ... 20-30 common patterns
    TaskIntent::Unknown
}
```

**Tier 2 — LLM fallback (local qwen3:4b):**

If keyword match fails, route to local inference with prompt:
```
You are hex, an AI Operating System. Classify this user intent into a task.
Intent: "{text}"
Respond with JSON: {"kind": "hex-command|workplan|shell", "payload": "..."}
Only use hex commands you know exist. If unsure, respond {"kind": "unknown"}.
```

Uses qwen3:4b (0.96 quality, 54 tok/s from today's bench) — essentially free.

### 3. Execution paths

**Immediate mode** (default):
```bash
hex hey calibrate inference
# → Classifies → Executes synchronously → Shows output
```

**Queue mode** (`--queue` flag):
```bash
hex hey --queue calibrate inference
# → Classifies → Enqueues to brain → Returns task ID
# → Brain daemon picks up on next tick
```

### 4. Confirmation for destructive operations

```bash
hex hey delete old worktrees
# → Classified as: hex worktree cleanup --force
# → Prompt: "This will remove N worktrees. Proceed? [y/N]"
# → --yes to skip confirmation
```

### 5. "Hey brain" alias

Optional wrapper script `hey-brain` that shells out to `hex hey`:
```bash
hey brain calibrate inference
```
For users who want the Siri-style feel.

## Consequences

**Positive:**
- Zero-friction task submission — just describe what you want
- Code-first classifier (tier 1) = no LLM cost for common intents
- LLM fallback (tier 2) handles edge cases with local model
- Pairs with brain daemon — `hex hey --queue X` = fire-and-forget

**Negative:**
- Intent classifier needs maintenance as commands evolve
- LLM fallback can be wrong — need explicit confirmation for destructive ops
- Natural language is ambiguous ("run X" could mean many things)

**Mitigations:**
- Tier 1 keyword match covers 80% of common intents deterministically
- Tier 2 fallback shows classified task before executing, user confirms
- Destructive operations always prompt

## Implementation

| Phase | Description | Status |
|-------|------------|--------|
| P1 | `hex hey <text>` command + keyword classifier (tier 1) | Pending |
| P2 | 20-30 common intent patterns | Pending |
| P3 | LLM fallback via local qwen3:4b for unknown intents | Pending |
| P4 | Confirmation prompts for destructive ops | Pending |
| P5 | `--queue` flag for async submission to brain daemon | Pending |
| P6 | `hey-brain` wrapper script | Pending |

## References

- ADR-2604132330: Brain Inbox Queue (execution target)
- ADR-2604132300: Brain Daemon Loop (async execution)
- ADR-2604131630: Code-First Execution (tier 1 keyword match, tier 2 LLM)

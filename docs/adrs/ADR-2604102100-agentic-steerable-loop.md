# ADR-2604102100: Agentic Steerable Loop

**Status:** Proposed
**Date:** 2026-04-10
**Drivers:** ADR-2604101900 steering API implemented but never polled; agents run once without checking for pending instructions
**Supersedes:** ADR-2604101900 (extends with runtime polling)

## Context

### Current State

ADR-2604101900 implemented the steering/interrupt API:
- `POST /api/steering/{id}/event` — stores pause/resume/restart events
- `POST /api/steering/{id}/interrupt` — stores new instructions
- `GET /api/steering/{id}/instructions` — polls for pending instructions

**The problem**: These endpoints store instructions but nothing checks them. The workplan executor spawns tasks that run to completion without ever polling for pending instructions. There's no agentic loop.

### What Others Do

| System | Steering Pattern |
|--------|-----------------|
| **PicoClaw** | After **each tool**, check steering queue. If messages: skip remaining tools, inject as user message, call LLM again. 4 polling points: loop start, after every tool, after LLM response, before turn finalized. |
| **OpenAI SDK** | Tool needs approval → returns `interruptions` in result → serialize `RunState` → `approve/reject()` → resume |
| **LangGraph** | `interrupt()` pauses graph → persist state → `Command(resume=...)` to continue |
| **Anthropic** | Continuous loop with `pause_turn`. Human interrupts via CLI (outside the loop) |

### Design Requirements

1. **Per-session instruction queue** — Instructions scoped to agent/session ID, not global
2. **Polling after operations** — Check after each task/phase completes
3. **Reaction to instructions** — pause → stop executing, resume → continue, interrupt → inject new instructions
4. **Serialization for resume** — Persist state so agents can resume after long pauses

### Alternatives Considered

1. **Add to workplan executor polling** — Simple but limited to workplan phases
2. **Build daemon loop in hex-nexus** — Full implementation but more complex
3. **Hybrid** — Workplan executor polls, separate daemon loop for continuous agents

## Decision

We will implement a **hybrid approach**:

1. **Phase 1**: Add instruction polling to workplan executor after each phase
2. **Phase 2**: Create a continuous agent loop in hex-cli that polls for both HexFlo tasks AND steering instructions
3. **State persistence**: Instructions are consumed (removed from queue) when polled, enabling resume

### Key Design Points

- **Polling endpoint**: `GET /api/steering/{agent_id}/instructions` returns pending instruction OR empty
- **One-time consumption**: Instructions are removed when polled (not peeked) — prevents re-executing
- **Polling intervals**: Every 5 seconds during active work, every 30 seconds when idle
- **Instruction types**:
  - `pause` — Stop current execution, preserve state for resume
  - `resume` — Resume paused execution (if state preserved)
  - `restart` — Clear state, start fresh with new instructions
  - `interrupt` — Stop, inject new instructions, continue

## Impact Analysis

### Consumer Dependency Map

| Artifact | Consumers | Impact | Mitigation |
|----------|-----------|--------|------------|
| `state.agent_instructions` | orchestration.rs handlers | HIGH - polling reads from same HashMap | Ensure thread-safe read/write |
| `session_poll_instructions` | CLI agent loop (new), workplan executor (new) | HIGH - will be called frequently | Add timeout/error handling |
| `/api/steering/{id}/instructions` | MCP tool (mcp.rs), CLI | MEDIUM - endpoint already exists | None needed |
| `InstructionType` enum | orchestration.rs handlers | LOW - extends existing | Add new variants |

### Cross-Crate Dependencies

- **hex-cli**: New agent loop that calls steering endpoints
- **hex-nexus**: Workplan executor polling (existing code)
- **MCP tools**: Already call `/api/steering/*` paths

### Blast Radius

| Component | Impact | Mitigation |
|-----------|--------|------------|
| state.rs `AgentInstruction` | MEDIUM - add queue/consume flag | Simple change |
| orchestration.rs handlers | MEDIUM - add polling logic | Test thoroughly |
| New CLI commands | LOW - additive | Document new commands |

## Build Verification Gates

| Gate | Command | Scope |
|------|--------|-------|
| Workspace compile | `cargo check --workspace` | All Rust crates |
| hex-cli compile | `cargo build -p hex-cli --release` | hex-cli binary |
| hex-nexus compile | `cargo build -p hex-nexus --release` | hex-nexus binary |
| API test | `curl http://localhost:5555/api/steering/test/instructions` | Steering endpoints |

## Consequences

**Positive:**
- Complete feature parity with Anthropic steering pattern
- Agents can be paused/resumed during long operations
- Human-in-the-loop becomes possible
- Clear separation between "run until done" and "run with oversight"

**Negative:**
- Additional complexity in workplan executor
- State management for pause/resume adds complexity
- Need to handle edge cases (agent dies while paused)

**Mitigations:**
- Document state persistence requirements
- Add health checks before resume
- Use HexFlo for agent state (already integrated)

## Implementation

| Phase | Description | Validation Gate | Status |
|-------|-------------|-----------------|--------|
| P1 | Add instruction polling to workplan executor after each phase | None (read-only) | Pending |
| P2 | Create state struct for agent execution (running/paused/complete) | `cargo check` | Pending |
| P3 | Implement CLI agent loop: poll HexFlo + poll steering, run until output | Build hex-cli | Pending |
| P4 | Add pause/resume/stop handlers in CLI loop | Manual test | Pending |
| P5 | End-to-end test: start task, pause mid-flight, resume, verify output | Run full pipeline | Pending |

## References

- ADR-2604101900: Steering/Interrupt API Implementation
- PicoClaw steering docs: https://docs.picoclaw.io/docs/steering
- OpenAI Agents SDK HITL: https://openai.github.io/openai-agents-js/guides/human-in-the-loop
- LangGraph interrupts: https://langchain-ai.github.io/langgraph/how-tos/human_in_the_loop/wait-user-input/
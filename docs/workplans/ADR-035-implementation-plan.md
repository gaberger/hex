# ADR-035 Implementation Plan — Hex Architecture V2

**Plan ID**: plan-1773920578572
**Steps**: 32
**Phases**: 8 (0–7)
**ADR**: [ADR-035](../adrs/ADR-035-hex-architecture-v2-rust-first-spacetime-native.md)

## Dependency Graph

```
Phase 0 (hex-core)          Phase 1a (STDB modules)       Phase 1b (inference)
┌────────────────┐          ┌──────────────────┐          ┌──────────────────┐
│ S1: Crate init │          │ S5: file-lock    │          │ S8:  Tables      │
│ S2: Domain     │─────────►│ S6: arch-enforce │          │ S9:  Reducer     │
│ S3: Ports      │          │ S7: conflict-res │          │ S10: Procedure   │
│ S4: Rules      │──────┐   └────────┬─────────┘          │ S11: Providers   │
└────────────────┘      │            │                     └────────┬─────────┘
                        │            │                              │
                        ▼            ▼                              ▼
Phase 2 (hex-agent)                          Phase 3 (hex-nexus)
┌────────────────────────────┐               ┌──────────────────────┐
│ S12: hex-core deps         │               │ S16: hex-core deps   │
│ S13: ValidatedCodeWriter   │◄──────────────│ S17: Port compose    │
│ S14: STDB InferenceAdapter │               │ S18: SDK subscriptions│
│ S15: Sandboxed FS          │               └──────────┬───────────┘
└────────────┬───────────────┘                          │
             │                                          │
             ▼                                          ▼
Phase 4 (hex-chat)                           Phase 5 (hex-cli)
┌────────────────────────────┐               ┌──────────────────────┐
│ S19: TUI panels            │               │ S23: Crate + clap    │
│ S20: Web dashboard (HTMX)  │               │ S24: Core commands   │
│ S21: STDB subscriber       │               │ S25: MCP server Rust │
│ S22: Chat relay / dispatch │               │ S26: npm wrapper     │
└────────────┬───────────────┘               └──────────┬───────────┘
             │                                          │
             ▼                                          ▼
Phase 6 (STDB-native hex-agent)              Phase 7 (Retire TS)
┌────────────────────────────┐               ┌──────────────────────┐
│ S27: STDB SDK replace HTTP │               │ S30: Remove TS src/  │
│ S28: Table subscriptions   │               │ S31: npm postinstall │
│ S29: Reducer heartbeat     │               │ S32: Integration tests│
└────────────────────────────┘               └──────────────────────┘
```

## Phase 0: Extract `hex-core` (Foundation)

**Goal**: Single source of truth for domain types and port traits across all crates.
**Risk**: Low — purely additive.
**Parallel**: None — everything depends on this.

| Step | Task | Adapter Boundary | Deps | Deliverable |
|------|------|-----------------|------|-------------|
| S1 | Create `hex-core/Cargo.toml` — zero runtime deps (`serde`, `thiserror`, `async-trait`) | secondary/filesystem | — | Compiling crate |
| S2 | Define shared domain types: `AgentId`, `AgentStatus`, `TaskId`, `TaskStatus`, `BoundaryRule`, `Violation`, `TokenPartition`, `FileLockClaim`, `ConflictResolution` | domain | S1 | `hex-core/src/domain/*.rs` |
| S3 | Define port traits: `ICoordinationPort`, `IInferencePort`, `IAnalysisPort`, `IFileSystemPort`, `ISecretPort` | ports | S2 | `hex-core/src/ports/*.rs` |
| S4 | Move hex architecture enforcement logic: `detect_layer()`, boundary rules, `Layer` enum | domain/validation | S3 | `hex-core/src/rules/{boundary,conflict}.rs` |

**Exit criteria**: `cd hex-core && cargo test` passes. All types compile. No runtime dependencies beyond serde/thiserror/async-trait.

---

## Phase 1a: SpacetimeDB Coordination Modules

**Goal**: Distributed file locking, pre-write boundary enforcement, conflict resolution.
**Risk**: Low — new modules, no existing code changes.
**Parallel**: S5, S6, S7 can run concurrently (independent modules).

| Step | Task | Adapter Boundary | Deps | Deliverable |
|------|------|-----------------|------|-------------|
| S5 | `file-lock-manager` module: `FileLock` table (`file_path` PK, `agent_id`, `lock_type`, `expires_at`), reducers: `acquire_lock`, `release_lock`, `expire_stale_locks` | secondary/coordination | S4 | Published to SpacetimeDB |
| S6 | `architecture-enforcer` module: `BoundaryRule` table, `WriteValidation` table, `validate_write` reducer (checks layer imports, writes verdict) | secondary/validation | S4 | Published to SpacetimeDB |
| S7 | `conflict-resolver` module: `ConflictEvent` table, `report_conflict` reducer (detects multi-agent file contention), `resolve_conflict` (priority-based or escalate) | secondary/coordination | S4 | Published to SpacetimeDB |

**Exit criteria**: All three modules publish to SpacetimeDB. Unit tests for each reducer. `spacetimedb call` exercises each reducer from CLI.

---

## Phase 1b: Inference Gateway Module

**Goal**: All LLM inference routes through SpacetimeDB procedures. API keys never leave the database.
**Risk**: Medium — SpacetimeDB procedures are beta.
**Parallel**: Can run concurrently with Phase 1a.

| Step | Task | Adapter Boundary | Deps | Deliverable |
|------|------|-----------------|------|-------------|
| S8 | Create `inference-gateway` module: `InferenceQueue` (scheduled table), `InferenceResponse`, `InferenceProvider`, `AgentBudget`, `InferenceStreamChunk` tables | secondary/llm | S4 | Table schemas deployed |
| S9 | `request_inference` reducer: validate `AgentBudget`, check `InferenceProvider.current_rpm`, select provider, insert into `InferenceQueue` with `ScheduleAt::Interval(Duration::ZERO)` | secondary/llm | S8 | Reducer accepts/rejects requests |
| S10 | `execute_inference` procedure: `ctx.http.fetch()` to LLM API, parse response, `ctx.with_tx()` write to `InferenceResponse` + update budget/rate counters | secondary/llm | S9 | Procedure makes HTTP calls |
| S11 | Provider-specific HTTP builders: `build_anthropic_request()`, `build_openai_compat_request()`, `build_ollama_request()`, `build_vllm_request()` + response parsers | secondary/llm | S10 | All 4 providers supported |

**Exit criteria**: Agent can call `request_inference` reducer → procedure calls Anthropic API → response appears in `InferenceResponse` table. `AgentBudget` correctly decremented. Rate limiting rejects over-quota requests.

---

## Phase 2: `hex-agent` Migrates to `hex-core`

**Goal**: hex-agent uses shared types, pre-write validation, and SpacetimeDB inference.
**Risk**: Medium — changes the write path and inference path.
**Parallel**: S12 first, then S13+S14+S15 concurrently.

| Step | Task | Adapter Boundary | Deps | Deliverable |
|------|------|-----------------|------|-------------|
| S12 | Add `hex-core` dependency, replace duplicate domain types (`AgentStatus`, `TaskStatus`, etc.) with `hex_core::domain::*` | secondary/filesystem | S4 | `hex-agent/Cargo.toml` updated, compiles |
| S13 | `ValidatedCodeWriter` usecase: acquire file lock (S5) → extract imports → call `validate_write` reducer (S6) → subscribe for verdict → write or reject | usecases/code-generator | S12, S5, S6 | Pre-write validation active |
| S14 | `SpacetimeInferenceAdapter`: implement `IInferencePort` by calling `request_inference` reducer, subscribe to `InferenceResponse`, use oneshot channels for async response delivery | secondary/llm | S12, S11 | Inference via SpacetimeDB |
| S15 | `SandboxedFsAdapter`: wrap file operations with `hex_core::rules::boundary::validate_imports()` before write | secondary/filesystem | S13 | Boundary-checked file I/O |

**Exit criteria**: hex-agent can write files with pre-validation. Inference calls go through SpacetimeDB. Boundary violations are rejected before write. All existing tests pass.

---

## Phase 3: `hex-nexus` Migrates to `hex-core`

**Goal**: Remove duplicate types, compose ports, use direct subscriptions.
**Risk**: Medium — refactors the central hub.
**Parallel**: S16 first, then S17+S18 concurrently.

| Step | Task | Adapter Boundary | Deps | Deliverable |
|------|------|-----------------|------|-------------|
| S16 | Add `hex-core` dependency, replace duplicate domain types with `hex_core::domain::*` | secondary/filesystem | S4 | Compiles with shared types |
| S17 | Refactor `IStatePort`: decompose into `ICoordinationPort` + `ISecretPort` (from hex-core) + nexus-specific extensions. Adapter wraps SpacetimeDB calls. | ports/coordination | S16 | Cleaner port boundaries |
| S18 | Replace HTTP polling with SpacetimeDB SDK WebSocket subscriptions for agent status, swarm state, task updates | secondary/coordination | S17, S7 | Real-time push, no polling |

**Exit criteria**: hex-nexus compiles with hex-core types. All REST endpoints still work. SpacetimeDB subscriptions deliver real-time updates.

---

## Phase 4: `hex-chat` Command Center

**Goal**: Standalone TUI + web dashboard for developer oversight.
**Risk**: Low — new crate, no existing code changes.
**Parallel**: S19+S20 concurrently, then S21, then S22.

| Step | Task | Adapter Boundary | Deps | Deliverable |
|------|------|-----------------|------|-------------|
| S19 | Create `hex-chat/Cargo.toml` (ratatui, crossterm, axum, spacetimedb-sdk). TUI panels: `fleet_panel` (agent list + status), `task_board` (kanban), `chat_panel` (messaging), `arch_panel` (violations), `token_gauge` (budget) | primary/dashboard | S4 | TUI renders with mock data |
| S20 | Web dashboard with axum + HTMX: same panels as TUI, served at `http://localhost:5556`. Replaces `hex-nexus/assets/chat.html` | primary/dashboard | S19 | Web dashboard serves |
| S21 | `SpacetimeSubscriberAdapter`: subscribe to `agent_registry`, `swarm_task`, `inference_response`, `write_validation`, `agent_budget` tables. Feed real-time data to TUI/web panels | primary/dashboard | S20, S18 | Live data in dashboard |
| S22 | Chat relay + order dispatch: developer sends `@agent-name <directive>` → writes to `chat_relay` via reducer → agent subscribes and receives | primary/hub-command | S21 | Bidirectional dev↔agent chat |

**Exit criteria**: `hex-chat` binary launches TUI or web dashboard. Real-time agent status, task progress, token budgets visible. Developer can message agents. Architecture violations appear live.

---

## Phase 5: `hex-cli` Rust Binary

**Goal**: Replace TypeScript CLI with thin Rust binary.
**Risk**: High — replaces the primary user interface.
**Parallel**: S23 first, then S24+S25 concurrently, then S26.

| Step | Task | Adapter Boundary | Deps | Deliverable |
|------|------|-----------------|------|-------------|
| S23 | Create `hex-cli/Cargo.toml` with `clap` (derive). Subcommands: `analyze`, `scaffold`, `build`, `plan`, `setup`, `adr`, `swarm`, `task`, `memory`, `daemon` | primary/cli | S4 | CLI parses all commands |
| S24 | Implement core commands: each delegates to hex-nexus REST API (same endpoints TS CLI used). `analyze` calls `/api/analyze`, `scaffold` calls `/api/scaffold`, etc. | primary/cli | S23 | Feature parity with TS CLI |
| S25 | MCP server in Rust: implement `hex_*` tool handlers (40+ tools). Uses `rmcp` or `mcp-rust-sdk`. Replaces `src/adapters/primary/mcp-adapter.ts` | primary/mcp | S24 | MCP tools work from Claude Code |
| S26 | npm package becomes binary wrapper: `postinstall` downloads platform-specific binary from GitHub releases. `bin/hex` shell script delegates to Rust binary | secondary/registry | S25 | `npx hex analyze .` still works |

**Exit criteria**: `hex-cli analyze .` produces same output as `bun src/cli.ts analyze .`. All 40+ MCP tools respond. npm package installs and delegates correctly.

---

## Phase 6: Direct SpacetimeDB SDK in `hex-agent`

**Goal**: Eliminate HTTP intermediary — hex-agent talks SpacetimeDB natively.
**Risk**: Medium — replaces the hub client adapter.
**Parallel**: S27 first, then S28+S29 concurrently.

| Step | Task | Adapter Boundary | Deps | Deliverable |
|------|------|-----------------|------|-------------|
| S27 | Replace `HubClientAdapter` (WebSocket to hex-nexus) with `SpacetimeDbClientAdapter` (direct SpacetimeDB SDK connection) | secondary/coordination | S15, S18 | Agent connects to SpacetimeDB directly |
| S28 | Subscribe to tables: `inference_response` (filtered by agent_id), `write_validation` (filtered by agent_id), `swarm_task` (filtered by swarm_id), `hexflo_memory` | secondary/coordination | S27 | Real-time push for all agent operations |
| S29 | Heartbeat via SpacetimeDB reducer: call `heartbeat(agent_id, status, turn_count, token_usage)` every 15s instead of HTTP POST to hex-nexus | secondary/coordination | S28 | No HTTP dependency for liveness |

**Exit criteria**: hex-agent operates without hex-nexus HTTP connection. All coordination via SpacetimeDB SDK. Heartbeat visible in `agent_heartbeat` table. Inference responses arrive via subscription.

---

## Phase 7: Retire TypeScript

**Goal**: Single-language codebase. npm package is a binary wrapper.
**Risk**: High — breaking change for existing workflows.
**Parallel**: S30 first, then S31+S32 concurrently.

| Step | Task | Adapter Boundary | Deps | Deliverable |
|------|------|-----------------|------|-------------|
| S30 | Remove `src/` directory: composition-root.ts, all adapters (primary + secondary), ports, usecases, domain. Keep `dist/` temporarily for backwards compat | secondary/filesystem | S26, S29, S22 | No TS source code |
| S31 | Update `package.json`: `postinstall` fetches platform binary (darwin-arm64, darwin-x64, linux-x64, linux-arm64). `bin.hex` points to wrapper script | secondary/registry | S30 | npm install downloads Rust binary |
| S32 | Final integration tests: all Rust crates compile together, SpacetimeDB modules deployed, hex-agent + hex-nexus + hex-chat + hex-cli all functional, MCP tools respond, `hex analyze .` on self passes | secondary/validation | S31 | Green CI across all crates |

**Exit criteria**: `cargo build --workspace` succeeds. `npm install hex` downloads and runs Rust binary. No TypeScript in the repository. All ADR-035 objectives met.

---

## Parallelism Map

```
Week 1-2:   [S1→S2→S3→S4]  Phase 0 (serial — foundation)
Week 3-4:   [S5 | S6 | S7]  Phase 1a (parallel — independent STDB modules)
            [S8→S9→S10→S11] Phase 1b (serial — inference gateway build-up)
Week 5-6:   [S12→S13 | S14 | S15]  Phase 2 (hex-agent migration)
            [S16→S17 | S18]         Phase 3 (hex-nexus migration)
Week 7-9:   [S19 | S20]→S21→S22    Phase 4 (hex-chat)
            [S23→S24 | S25]→S26    Phase 5 (hex-cli)
Week 10-11: [S27→S28 | S29]        Phase 6 (STDB-native agent)
Week 12:    [S30→S31 | S32]        Phase 7 (retire TS)
```

**Critical path**: S1→S2→S3→S4→S12→S13→S15→S27→S28→S30→S32

## Risk Register

| Risk | Impact | Likelihood | Mitigation |
|------|--------|-----------|------------|
| SpacetimeDB procedures beta API changes | S10, S14 break | Medium | Abstract behind IInferencePort — swap to direct HTTP adapter |
| SpacetimeDB streaming not supported in procedures | S11 streaming unusable | Medium | Fallback: direct SSE for interactive, STDB for batch |
| Rust MCP SDK immature | S25 blocked | Low | Use `rmcp` crate (active, well-maintained) or raw JSON-RPC |
| npm binary distribution complexity | S26, S31 fragile | Medium | Follow esbuild/turbo pattern — proven at scale |
| hex-agent/hex-nexus type divergence during migration | S12, S16 conflict | Low | hex-core PR merges first, both crates update in same sprint |
| TUI complexity (ratatui learning curve) | S19 slow | Medium | Start with web dashboard (S20), TUI as progressive enhancement |
| Rate limit window drift in STDB | S9 inaccurate | Low | Scheduled reducer resets every 60s; good enough for ±1 request |

## Success Metrics

| Metric | Current | Target |
|--------|---------|--------|
| Languages in repo | 2 (TS + Rust) | 1 (Rust) |
| Binary count | 3 + Node.js | 4 Rust binaries |
| Agent→LLM call path | Agent → HTTP → hex-nexus → HTTP → Anthropic | Agent → STDB reducer → procedure → Anthropic |
| Boundary violation detection | Post-hoc (`hex analyze`) | Pre-write (STDB enforcer) |
| Rate limit handling | Per-agent retry loops | Centralized admission control |
| API key exposure | Environment variables in agent processes | Keys stay in STDB `secret_vault` |
| Agent coordination latency | HTTP polling (100-500ms) | STDB subscription push (~5ms) |
| Developer visibility | Debug chat.html | Full TUI + web command center |

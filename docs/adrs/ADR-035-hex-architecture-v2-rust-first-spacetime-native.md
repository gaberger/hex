# ADR-035: Hex Architecture V2 — Rust-First, SpacetimeDB-Native, Pluggable Inference

- **Status**: Proposed
- **Date**: 2026-03-19
- **Supersedes**: Partial aspects of ADR-024, ADR-025, ADR-027, ADR-032, ADR-034
- **Authors**: Gary (architect), Claude (adversarial analysis)

## Context

hex has grown organically from a TypeScript CLI tool with Rust backends into a multi-crate system spanning two languages, three binaries, and 10 SpacetimeDB modules. The current architecture works but has structural tension:

| Problem | Evidence |
|---------|----------|
| **Dual-language overhead** | TypeScript CLI (`src/`) wraps Rust binaries (`hex-nexus`, `hex-agent`) — 20+ adapters on TS side mirror Rust ports |
| **Composition root sprawl** | `composition-root.ts` is 300+ lines of wiring that mostly launches a Rust binary and proxies HTTP |
| **Hub lifecycle fragility** | TS launches hex-nexus as a child process, polls lock files, compares build hashes — failure modes at every step |
| **Claude Code coupling** | hex-agent's conversation loop reimplements Claude Code's agent loop rather than delegating to it |
| **SpacetimeDB underutilized** | 10 modules deployed but coordination still happens through HTTP REST + in-memory state in `AppState` |
| **No true multi-agent safety** | HexFlo tracks tasks but doesn't prevent file conflicts, merge races, or architectural violations at write-time |
| **Chat is an afterthought** | `chat.html` is a debug tool embedded in the binary, not a production command interface |

The user's vision: hex is a **Rust-native coordination framework** where SpacetimeDB is the **nervous system** connecting hundreds of agents, inference engines are **pluggable adapters**, and hex-chat is the **developer's command center** — a CEO interface over a workforce of AI agents.

## Decision

Restructure hex into **four Rust crates** with clear boundaries, a **SpacetimeDB-native coordination plane**, and **pluggable inference adapters** that treat Claude Code, hex-agent, and any LLM backend as interchangeable workers.

### Architecture Overview

```
┌─────────────────────────────────────────────────────────────────────┐
│                        hex-chat (Binary)                           │
│           Developer Command Center — TUI + Web Dashboard           │
│  ┌──────────┐  ┌──────────┐  ┌───────────┐  ┌───────────────────┐ │
│  │ Agent    │  │ Token    │  │ Task      │  │ Architecture      │ │
│  │ Fleet    │  │ Budget   │  │ Board     │  │ Compliance View   │ │
│  │ Monitor  │  │ Tracker  │  │ (Kanban)  │  │ (Live Violations) │ │
│  └──────────┘  └──────────┘  └───────────┘  └───────────────────┘ │
│           ↕ SpacetimeDB subscriptions (real-time push)             │
├─────────────────────────────────────────────────────────────────────┤
│                    SpacetimeDB (Coordination Plane)                 │
│  ┌────────────┐ ┌────────────┐ ┌─────────────┐ ┌───────────────┐  │
│  │ Agent      │ │ Task       │ │ File Lock   │ │ Architecture  │  │
│  │ Registry   │ │ Orchestr.  │ │ Manager     │ │ Enforcer      │  │
│  ├────────────┤ ├────────────┤ ├─────────────┤ ├───────────────┤  │
│  │ RL Engine  │ │ Chat Relay │ │ Secret Vault│ │ Conflict Res. │  │
│  └────────────┘ └────────────┘ └─────────────┘ └───────────────┘  │
│           ↕ SpacetimeDB SDK (WebSocket subscriptions)              │
├─────────────┬──────────────┬──────────────┬────────────────────────┤
│ hex-agent   │ Claude Code  │ Cursor/etc.  │  Any MCP Client       │
│ (Native)    │ (Adapter)    │ (Adapter)    │  (Adapter)            │
│             │              │              │                        │
│ Anthropic ──┤              │              │                        │
│ MiniMax  ──┤  Inference   │  Inference   │  Inference             │
│ Ollama   ──┤  via host    │  via host    │  via host              │
│ vLLM     ──┤              │              │                        │
└─────────────┴──────────────┴──────────────┴────────────────────────┘
```

### Crate Structure

```
hex-intf/
├── hex-core/              # NEW — Shared domain + ports (library crate)
│   ├── domain/            # Value objects, entities, hex rules
│   │   ├── architecture.rs   # Layer enum, BoundaryRule, Violation
│   │   ├── agent.rs          # AgentId, AgentStatus, Heartbeat
│   │   ├── task.rs           # TaskId, TaskStatus, WorkplanPhase
│   │   ├── file_lock.rs      # FileLockClaim, ConflictResolution
│   │   └── token_budget.rs   # TokenPartition, UsageMetrics
│   ├── ports/             # Trait definitions (contracts)
│   │   ├── coordination.rs   # ICoordinationPort (SpacetimeDB abstraction)
│   │   ├── inference.rs      # IInferencePort (pluggable LLM backend)
│   │   ├── analysis.rs       # IAnalysisPort (tree-sitter, boundary check)
│   │   ├── file_system.rs    # IFileSystemPort (sandboxed file ops)
│   │   └── secret.rs         # ISecretPort (vault access)
│   └── rules/             # Hex architecture enforcement logic
│       ├── boundary.rs       # Import validation, layer enforcement
│       └── conflict.rs       # File lock arbitration, merge strategy
│
├── hex-nexus/             # EVOLVED — Orchestration + SpacetimeDB native
│   ├── adapters/
│   │   ├── spacetime_coordination.rs  # ICoordinationPort via SpacetimeDB
│   │   ├── spacetime_secrets.rs       # ISecretPort via SpacetimeDB
│   │   └── tree_sitter_analysis.rs    # IAnalysisPort via tree-sitter
│   ├── coordination/      # HexFlo v2 — SpacetimeDB-native
│   │   ├── file_locks.rs     # File-level locking via SpacetimeDB reducers
│   │   ├── conflict_resolver.rs  # Automatic conflict resolution
│   │   └── arch_enforcer.rs     # Pre-write boundary validation
│   ├── orchestration/     # Agent lifecycle + workplan execution
│   ├── routes/            # HTTP/WS API (for non-SpacetimeDB clients)
│   └── bin/hex-nexus.rs   # Binary entry
│
├── hex-agent/             # EVOLVED — Autonomous code agent
│   ├── domain/            # Agent-specific domain (tools, knowledge, scoring)
│   ├── ports/             # Agent-specific ports (inference, RL, tools)
│   ├── adapters/
│   │   ├── anthropic.rs          # IInferencePort → Anthropic API
│   │   ├── openai_compat.rs      # IInferencePort → OpenAI-compatible
│   │   ├── claude_code_bridge.rs # NEW — IInferencePort → Claude Code MCP
│   │   ├── sandboxed_fs.rs       # IFileSystemPort with hex boundary checks
│   │   └── spacetime_rl.rs       # IRlPort → SpacetimeDB RL engine
│   ├── usecases/
│   │   ├── conversation.rs       # Multi-turn loop with RL-driven model selection
│   │   ├── context_packer.rs     # System prompt assembly with hex knowledge
│   │   └── code_writer.rs        # NEW — File write with pre-write validation
│   └── bin/hex-agent.rs
│
├── hex-chat/              # NEW — Developer Command Center
│   ├── tui/               # Terminal UI (ratatui)
│   │   ├── fleet_panel.rs    # Agent fleet monitor (status, tokens, tasks)
│   │   ├── task_board.rs     # Kanban-style task board
│   │   ├── chat_panel.rs    # Direct agent communication
│   │   ├── arch_panel.rs    # Live architecture compliance
│   │   └── token_gauge.rs   # Budget tracking across all agents
│   ├── web/               # Web dashboard (axum + HTMX)
│   │   └── (replaces current chat.html)
│   ├── adapters/
│   │   ├── spacetime_subscriber.rs  # Real-time push via SpacetimeDB SDK
│   │   └── nexus_client.rs          # HTTP fallback for legacy clients
│   └── bin/hex-chat.rs
│
├── spacetime-modules/     # EXPANDED — The nervous system
│   ├── agent-registry/
│   ├── task-orchestration/    # Renamed from workplan-state
│   ├── file-lock-manager/     # NEW — Distributed file locking
│   ├── architecture-enforcer/ # NEW — Pre-write boundary validation
│   ├── conflict-resolver/     # NEW — Multi-agent conflict arbitration
│   ├── rl-engine/
│   ├── secret-grant/
│   ├── chat-relay/
│   ├── hexflo-coordination/
│   ├── fleet-state/
│   ├── skill-registry/
│   ├── hook-registry/
│   └── agent-definition-registry/
│
└── hex-cli/               # SLIMMED — Thin Rust CLI (replaces TS CLI)
    └── (delegates everything to hex-nexus API + hex-chat TUI)
```

### Key Architectural Decisions

#### 1. Extract `hex-core` as Shared Domain Library

**Why**: hex-nexus and hex-agent both define overlapping domain types (`AgentStatus`, `TaskStatus`, `SwarmInfo`). The TS side mirrors these again. One source of truth eliminates drift.

**Rule**: `hex-core` has **zero runtime dependencies** — only `serde`, `thiserror`, `async-trait`. Every other crate depends on `hex-core` for shared types and port traits.

```toml
# hex-core/Cargo.toml
[dependencies]
serde = { version = "1", features = ["derive"] }
thiserror = "2"
async-trait = "0.1"
# Nothing else. Ever.
```

#### 2. SpacetimeDB as the Coordination Plane (Not Just Storage)

**Current state**: SpacetimeDB stores data, but coordination still happens via HTTP REST calls and in-memory `AppState` (RwLock<HashMap>). Agents poll for tasks.

**Target state**: SpacetimeDB **is** the coordination plane. Agents subscribe to table changes via the SpacetimeDB SDK's WebSocket subscriptions. No polling. No REST intermediary for agent-to-agent coordination.

```
Current:  Agent → HTTP POST hex-nexus → hex-nexus writes SpacetimeDB → other agents poll
Target:   Agent → SpacetimeDB reducer → subscription pushes to all subscribers instantly
```

**New SpacetimeDB Modules**:

##### `file-lock-manager` — Distributed File Locking
```rust
#[spacetimedb::table(public)]
struct FileLock {
    #[primary_key]
    file_path: String,
    agent_id: String,
    lock_type: String,       // "exclusive" | "shared_read"
    acquired_at: String,
    expires_at: String,       // 5 min TTL, renewable
    worktree: Option<String>, // Which worktree holds the lock
}

#[spacetimedb::reducer]
fn acquire_lock(ctx: &ReducerContext, file_path: String, agent_id: String, lock_type: String) {
    // Check for conflicts BEFORE granting
    // Exclusive locks block all others
    // Shared reads allow concurrent reads
    // Returns error if conflicting lock exists
}

#[spacetimedb::reducer]
fn release_lock(ctx: &ReducerContext, file_path: String, agent_id: String) { ... }

#[spacetimedb::reducer]
fn expire_stale_locks(ctx: &ReducerContext) {
    // Called by cleanup timer — releases locks from dead agents
}
```

##### `architecture-enforcer` — Pre-Write Boundary Validation
```rust
#[spacetimedb::table(public)]
struct BoundaryRule {
    #[primary_key]
    rule_id: String,
    source_layer: String,     // "adapters/primary"
    forbidden_import: String, // "adapters/secondary"
    severity: String,         // "error" | "warning"
}

#[spacetimedb::table(public)]
struct WriteValidation {
    #[primary_key]
    validation_id: String,
    agent_id: String,
    file_path: String,
    proposed_imports: String,  // JSON array
    verdict: String,           // "approved" | "rejected"
    violations: String,        // JSON array of violated rules
    validated_at: String,
}

#[spacetimedb::reducer]
fn validate_write(ctx: &ReducerContext, agent_id: String, file_path: String, proposed_imports: String) {
    // Check all boundary rules
    // Record verdict in WriteValidation table
    // Agent subscribes to WriteValidation and checks verdict before committing
}
```

##### `conflict-resolver` — Multi-Agent Conflict Arbitration
```rust
#[spacetimedb::table(public)]
struct ConflictEvent {
    #[primary_key]
    conflict_id: String,
    file_path: String,
    agents: String,            // JSON array of competing agent_ids
    resolution: String,        // "priority" | "merge" | "escalate"
    resolved_by: String,       // agent_id or "system"
    created_at: String,
    resolved_at: Option<String>,
}

#[spacetimedb::reducer]
fn report_conflict(ctx: &ReducerContext, file_path: String, agents: String) {
    // Detect: same file modified by 2+ agents in different worktrees
    // Strategy: higher-tier task wins (domain > adapter), else escalate to hex-chat
}
```

#### 3. Claude Code as an Inference Adapter (Not the Runtime)

**Current**: hex-agent reimplements Claude Code's conversation loop. Claude Code is the host that runs hex as an MCP server.

**Target**: Claude Code is **one of many** inference adapters. hex-agent can use Claude Code's capabilities when available, but also works with Anthropic API directly, MiniMax, Ollama, vLLM, or any OpenAI-compatible endpoint.

```rust
// hex-core/ports/inference.rs
#[async_trait]
pub trait IInferencePort: Send + Sync {
    /// Send a message and get a response (may include tool_use)
    async fn complete(&self, request: InferenceRequest) -> Result<InferenceResponse>;

    /// Stream a response chunk by chunk
    async fn stream(&self, request: InferenceRequest) -> Result<Pin<Box<dyn Stream<Item = StreamChunk>>>>;

    /// What models does this backend support?
    fn available_models(&self) -> Vec<ModelCapability>;

    /// Does this backend support tool_use natively?
    fn supports_tool_use(&self) -> bool;

    /// Does this backend support extended thinking?
    fn supports_thinking(&self) -> bool;
}

// hex-agent/adapters/claude_code_bridge.rs
/// Adapter that delegates inference to a running Claude Code instance via MCP
pub struct ClaudeCodeBridge {
    mcp_client: McpStdioClient,
}

impl IInferencePort for ClaudeCodeBridge {
    // Translates hex-agent tool calls into Claude Code tool calls
    // Receives results back through MCP protocol
    // Claude Code handles the actual LLM interaction
}
```

**Adapter hierarchy** (RL engine selects based on task):

| Adapter | When to Use | Cost | Latency |
|---------|------------|------|---------|
| `AnthropicDirect` | Default, full control | $$$ | ~2s |
| `ClaudeCodeBridge` | User has Claude Code running, wants its context | $$$ | ~3s |
| `MiniMaxAdapter` | Budget tasks, high volume | $ | ~1s |
| `OllamaAdapter` | Local, air-gapped, unlimited | Free | ~5s |
| `VllmAdapter` | Self-hosted GPU cluster | Infra | ~1s |

#### 4. hex-chat as Developer Command Center

**Current**: `chat.html` is a debug WebSocket client embedded in hex-nexus.

**Target**: `hex-chat` is a **standalone binary** — both a TUI (terminal) and web dashboard — that gives the developer CEO-level visibility and control over the entire agent workforce.

```
┌─────────────────────────────────────────────────────────────┐
│ hex-chat v2                                    ⚡ 12 agents │
├──────────────┬──────────────────────┬────────────────────────┤
│ FLEET        │ TASK BOARD           │ CHAT                   │
│              │                      │                        │
│ ● opus-1     │ ▓▓▓▓▓▓▓░░░ 70%     │ > @opus-1 refactor the │
│   domain/    │ feat: auth-module    │   auth adapter to use  │
│   142K tok   │                      │   the new ISecretPort  │
│              │ ┌──────┐ ┌────────┐ │                        │
│ ● sonnet-2   │ │TODO  │ │IN PROG │ │ opus-1: I'll start by  │
│   adapters/  │ │ T-04 │ │ T-01 ● │ │ reading the current... │
│   87K tok    │ │ T-05 │ │ T-02 ● │ │                        │
│              │ │      │ │ T-03 ● │ │ [file_lock acquired:   │
│ ○ haiku-3    │ └──────┘ └────────┘ │  auth_adapter.rs]      │
│   idle       │                      │                        │
│              │ ┌──────┐            │ ⚠ VIOLATION: T-02      │
│ TOKENS       │ │DONE  │            │ sonnet-2 importing     │
│ Total: 892K  │ │ T-00 ✓│            │ from adapters/primary  │
│ Budget: 2M   │ └──────┘            │ → BLOCKED, fix needed  │
│ ▓▓▓▓░░░ 45%  │                      │                        │
├──────────────┴──────────────────────┴────────────────────────┤
│ ARCH: 0 violations │ LOCKS: 3 active │ RL: ε=0.08 avg=0.72  │
└─────────────────────────────────────────────────────────────┘
```

**Key capabilities**:
- **Fleet Monitor**: All agents, their status, token usage, current task, heartbeat
- **Task Board**: Kanban view of workplan phases (TODO → In Progress → Done)
- **Chat Panel**: Direct messaging to any agent ("@opus-1 stop and explain your approach")
- **Architecture Compliance**: Live violations feed — blocks agents that break hex rules
- **Token Budget**: Aggregate budget tracking across all agents, with per-agent drill-down
- **RL Dashboard**: Epsilon-greedy exploration rate, average reward, model distribution
- **Conflict Alerts**: Real-time notification when agents contend for same files
- **Order Dispatch**: Developer sends high-level directives ("focus on domain layer first, then fan out")

**Data source**: SpacetimeDB subscriptions (real-time push, not polling).

#### 5. Retire TypeScript CLI — Replace with Rust `hex-cli`

**Why**: The TS CLI exists primarily to launch hex-nexus and proxy commands. With hex-nexus running as a daemon and hex-chat as the developer interface, the CLI becomes a thin command dispatcher.

**Migration path**:

| Phase | Action | Timeline |
|-------|--------|----------|
| 1 | Create `hex-cli` Rust crate with clap | Week 1 |
| 2 | Implement core commands (`analyze`, `scaffold`, `build`, `plan`) calling hex-nexus API | Week 2-3 |
| 3 | Move MCP server to `hex-cli` (Rust MCP SDK) | Week 4-5 |
| 4 | Deprecate TS `src/` directory, keep `dist/` for backwards compat | Week 6 |
| 5 | Remove TS entirely, publish `hex-cli` as standalone binary | Week 8 |

**What stays in TS**: Nothing. The npm package becomes a wrapper that downloads and runs the Rust binary (like `esbuild` or `turbo` does).

#### 6. Sandboxed File Operations with Pre-Write Validation

**Current**: hex-agent executes `write_file` and `edit_file` directly. Boundary violations are caught by post-hoc analysis (`hex analyze`).

**Target**: Every file write goes through a **pre-write validation pipeline**:

```
Agent wants to write file
  → Acquire file lock (SpacetimeDB file-lock-manager)
  → Submit proposed imports to architecture-enforcer reducer
  → Wait for WriteValidation subscription event
  → If "approved": write file, release lock
  → If "rejected": report violation to hex-chat, agent self-corrects
```

```rust
// hex-agent/usecases/code_writer.rs
pub struct ValidatedCodeWriter {
    fs: Arc<dyn IFileSystemPort>,
    coordination: Arc<dyn ICoordinationPort>,
    analysis: Arc<dyn IAnalysisPort>,
}

impl ValidatedCodeWriter {
    pub async fn write_file(&self, path: &str, content: &str) -> Result<WriteResult> {
        // 1. Acquire lock
        self.coordination.acquire_file_lock(path, &self.agent_id).await?;

        // 2. Extract imports from proposed content
        let imports = self.analysis.extract_imports(content)?;

        // 3. Validate boundary rules
        let validation = self.coordination.validate_write(path, &imports).await?;

        match validation.verdict {
            Verdict::Approved => {
                self.fs.write(path, content).await?;
                self.coordination.release_file_lock(path, &self.agent_id).await?;
                Ok(WriteResult::Written)
            }
            Verdict::Rejected(violations) => {
                self.coordination.release_file_lock(path, &self.agent_id).await?;
                Err(HexError::BoundaryViolation(violations))
            }
        }
    }
}
```

#### 7. RL Engine Stays in SpacetimeDB — Agents Subscribe to Decisions

**Current**: Agent HTTP-calls hub for `select_action()`, hub queries SpacetimeDB RL module, returns action.

**Target**: Agent subscribes to SpacetimeDB `rl_q_entry` table directly. Model selection happens **client-side** using Q-values pushed via subscription. Reward reporting calls reducers directly.

```rust
// hex-agent/adapters/spacetime_rl.rs
pub struct SpacetimeRlAdapter {
    q_table: DashMap<String, f64>,  // Populated by SpacetimeDB subscription
    epsilon: f64,
}

impl IRlPort for SpacetimeRlAdapter {
    async fn select_action(&self, state: &RlState) -> RlAction {
        // Q-values already local via subscription — no HTTP call
        let state_key = state.to_key();
        epsilon_greedy_select(&self.q_table, &state_key, self.epsilon)
    }

    async fn report_reward(&self, reward: &RlReward) {
        // Direct SpacetimeDB reducer call — no HTTP intermediary
        self.stdb.call_reducer("record_reward", reward).await;
    }
}
```

### SpacetimeDB Function Architecture for Hex Enforcement

The user specifically called out [SpacetimeDB functions](https://spacetimedb.com/docs/functions) as the mechanism for enforcing hex principles. Here's how reducers enforce architecture:

```rust
// spacetime-modules/architecture-enforcer/src/lib.rs

/// Called before any agent writes a file — the gatekeeper
#[spacetimedb::reducer]
fn validate_write(
    ctx: &ReducerContext,
    agent_id: String,
    file_path: String,
    proposed_imports_json: String,
) {
    let imports: Vec<String> = serde_json::from_str(&proposed_imports_json).unwrap();
    let layer = detect_layer(&file_path);

    let mut violations = Vec::new();

    for import in &imports {
        let import_layer = detect_layer(import);

        // Rule 1: domain/ must only import from domain/
        if layer == Layer::Domain && import_layer != Layer::Domain {
            violations.push(format!("domain/ cannot import from {}", import_layer));
        }

        // Rule 2: ports/ may import from domain/ only
        if layer == Layer::Ports && import_layer != Layer::Domain && import_layer != Layer::Ports {
            violations.push(format!("ports/ cannot import from {}", import_layer));
        }

        // Rule 6: adapters must NEVER import other adapters
        if layer.is_adapter() && import_layer.is_adapter() && layer != import_layer {
            violations.push(format!("cross-adapter import: {} → {}", layer, import_layer));
        }
    }

    let verdict = if violations.is_empty() { "approved" } else { "rejected" };

    // Insert validation result — agent is subscribed, gets instant notification
    ctx.db.write_validation().insert(WriteValidation {
        validation_id: format!("{}:{}", agent_id, ctx.timestamp),
        agent_id,
        file_path,
        proposed_imports: proposed_imports_json,
        verdict: verdict.into(),
        violations: serde_json::to_string(&violations).unwrap(),
        validated_at: ctx.timestamp.to_string(),
    });
}
```

This moves enforcement from **post-hoc analysis** to **pre-write gatekeeping** — violations are impossible, not merely detectable.

### Inference Engine Pluggability

```rust
// hex-core/ports/inference.rs

/// Every inference backend implements this
#[async_trait]
pub trait IInferencePort: Send + Sync {
    async fn complete(&self, req: InferenceRequest) -> Result<InferenceResponse>;
    async fn stream(&self, req: InferenceRequest) -> Result<BoxStream<StreamChunk>>;
    fn capabilities(&self) -> InferenceCapabilities;
}

pub struct InferenceCapabilities {
    pub models: Vec<ModelInfo>,
    pub supports_tool_use: bool,
    pub supports_thinking: bool,
    pub supports_caching: bool,
    pub supports_streaming: bool,
    pub max_context_tokens: u64,
    pub cost_per_mtok_input: f64,
    pub cost_per_mtok_output: f64,
}

/// The RL engine uses capabilities + cost to select the best backend for each task
pub struct ModelInfo {
    pub id: String,
    pub provider: String,     // "anthropic", "minimax", "ollama", "vllm"
    pub tier: ModelTier,       // Opus, Sonnet, Haiku, Local
    pub context_window: u64,
}
```

**Adapter implementations**:

| Adapter | Backend | Registration |
|---------|---------|-------------|
| `AnthropicAdapter` | Anthropic Messages API | `ANTHROPIC_API_KEY` |
| `OpenAiCompatAdapter` | MiniMax, Together, Groq, OpenRouter | Discovered via SpacetimeDB `inference_endpoint` |
| `OllamaAdapter` | Local Ollama instance | Auto-discovered on localhost:11434 |
| `VllmAdapter` | Self-hosted vLLM | Registered via fleet-state |
| `ClaudeCodeBridge` | Running Claude Code process | Detected via MCP handshake |

The RL engine's `select_action()` now returns both a **model** and a **backend**, enabling cross-provider optimization.

### 8. SpacetimeDB Procedures as the Inference Gateway

**Key insight**: SpacetimeDB distinguishes between **reducers** (isolated, no network, transactional) and **procedures** (can make HTTP calls via `ctx.http`, non-transactional, use `ctx.with_tx()` for atomic DB writes). See [SpacetimeDB Procedures](https://spacetimedb.com/docs/functions/procedures).

This means inference calls can flow **through** SpacetimeDB itself — not just be coordinated by it. The database becomes the **inference gateway**.

#### Architecture: Request → Schedule → Procedure → Response

```
Agent calls reducer                     SpacetimeDB internally              Agent receives
─────────────────                       ────────────────────────            ─────────────────
request_inference(                      schedule table fires                subscription push
  prompt, model,          ──►           execute_inference()     ──►        inference_response
  agent_id, budget                        ctx.http.fetch(api)              table row arrives
)                                         ctx.with_tx(write)

Reducer validates budget                Procedure makes HTTP call           Agent reads response
& writes to schedule table              to Anthropic/MiniMax/Ollama         and continues work
(instant, ScheduleAt=0)                 & writes response atomically
```

#### New SpacetimeDB Module: `inference-gateway`

```rust
// spacetime-modules/inference-gateway/src/lib.rs

use spacetimedb::{table, reducer, ReducerContext, ProcedureContext, ScheduleAt};
use std::time::Duration;

// ─── Tables ───────────────────────────────────────────────

/// Inference request queue — agents write here, procedures consume
#[table(accessor = inference_queue, scheduled(execute_inference))]
pub struct InferenceQueue {
    #[primary_key]
    #[auto_inc]
    pub request_id: u64,
    pub scheduled_at: ScheduleAt,
    pub agent_id: String,
    pub provider: String,          // "anthropic", "minimax", "ollama", "vllm"
    pub model: String,             // "claude-sonnet-4-20250514", "MiniMax-M1"
    pub messages_json: String,     // Serialized Vec<Message>
    pub tools_json: String,        // Serialized Vec<ToolDefinition>
    pub max_tokens: u32,
    pub temperature: f32,
    pub thinking_budget: Option<u32>,
    pub cache_control: bool,
    pub priority: u8,              // 0=low, 1=normal, 2=high, 3=critical
    pub created_at: String,
}

/// Inference responses — agents subscribe to this table filtered by agent_id
#[table(public)]
pub struct InferenceResponse {
    #[primary_key]
    #[auto_inc]
    pub response_id: u64,
    pub request_id: u64,           // Links back to InferenceQueue
    pub agent_id: String,
    pub status: String,            // "completed", "failed", "rate_limited"
    pub content_json: String,      // Serialized response (text + tool_use blocks)
    pub model_used: String,        // Actual model that served the request
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
    pub latency_ms: u64,
    pub cost_usd: f64,             // Computed from token counts + model pricing
    pub created_at: String,
}

/// Provider registry — discovered endpoints with health status
#[table(public)]
pub struct InferenceProvider {
    #[primary_key]
    pub provider_id: String,
    pub provider_type: String,     // "anthropic", "openai_compat", "ollama", "vllm"
    pub base_url: String,
    pub api_key_ref: String,       // Reference to secret_vault key (never plaintext)
    pub models_json: String,       // Available models + capabilities
    pub rate_limit_rpm: u32,       // Requests per minute
    pub rate_limit_tpm: u64,       // Tokens per minute
    pub current_rpm: u32,          // Rolling window counter
    pub current_tpm: u64,
    pub healthy: bool,
    pub last_health_check: String,
    pub avg_latency_ms: u64,
}

/// Per-agent token budget enforcement
#[table(public)]
pub struct AgentBudget {
    #[primary_key]
    pub agent_id: String,
    pub total_budget_tokens: u64,
    pub used_tokens: u64,
    pub total_budget_usd: f64,
    pub used_usd: f64,
    pub max_single_request_tokens: u64,
    pub updated_at: String,
}

// ─── Reducer: Queue an inference request ──────────────────

/// Agents call this reducer to request inference.
/// The reducer validates budget, selects provider, and queues the request.
/// It CANNOT make HTTP calls (reducer isolation) — that's the procedure's job.
#[reducer]
fn request_inference(
    ctx: &ReducerContext,
    agent_id: String,
    model: String,
    messages_json: String,
    tools_json: String,
    max_tokens: u32,
    temperature: f32,
    thinking_budget: Option<u32>,
    cache_control: bool,
    priority: u8,
) {
    // 1. Budget enforcement — reject if agent is over budget
    let budget = ctx.db.agent_budget()
        .filter(|b| b.agent_id == agent_id)
        .next();

    if let Some(budget) = &budget {
        let estimated_tokens = max_tokens as u64 + estimate_input_tokens(&messages_json);
        if budget.used_tokens + estimated_tokens > budget.total_budget_tokens {
            // Write a "budget_exceeded" response immediately — no HTTP call needed
            ctx.db.inference_response().insert(InferenceResponse {
                response_id: 0,
                request_id: 0,
                agent_id: agent_id.clone(),
                status: "budget_exceeded".into(),
                content_json: r#"{"error":"Token budget exceeded"}"#.into(),
                model_used: model.clone(),
                input_tokens: 0, output_tokens: 0,
                cache_read_tokens: 0, cache_write_tokens: 0,
                latency_ms: 0, cost_usd: 0.0,
                created_at: timestamp_now(),
            });
            return;
        }
    }

    // 2. Provider selection — pick healthiest provider for this model
    let provider = select_provider(ctx, &model);

    // 3. Rate limit check — reject if provider is at capacity
    if let Some(ref p) = provider {
        if p.current_rpm >= p.rate_limit_rpm {
            // Try fallback provider or write rate_limited response
            ctx.db.inference_response().insert(InferenceResponse {
                response_id: 0, request_id: 0,
                agent_id: agent_id.clone(),
                status: "rate_limited".into(),
                content_json: format!(r#"{{"error":"Rate limited on {}"}}"#, p.provider_id),
                model_used: model.clone(),
                input_tokens: 0, output_tokens: 0,
                cache_read_tokens: 0, cache_write_tokens: 0,
                latency_ms: 0, cost_usd: 0.0,
                created_at: timestamp_now(),
            });
            return;
        }
    }

    // 4. Queue the request — procedure fires immediately (ScheduleAt = 0)
    ctx.db.inference_queue().insert(InferenceQueue {
        request_id: 0, // auto_inc
        scheduled_at: ScheduleAt::Interval(Duration::ZERO.into()),
        agent_id,
        provider: provider.map(|p| p.provider_id).unwrap_or("anthropic".into()),
        model,
        messages_json,
        tools_json,
        max_tokens,
        temperature,
        thinking_budget,
        cache_control,
        priority,
        created_at: timestamp_now(),
    });
}

// ─── Procedure: Execute the inference call ────────────────

/// Scheduled procedure — fires immediately when a row is inserted into inference_queue.
/// Has access to ctx.http for outbound HTTP calls.
/// Runs OUTSIDE a transaction — uses ctx.with_tx() for atomic DB writes.
#[spacetimedb::procedure]
fn execute_inference(ctx: &mut ProcedureContext, request: InferenceQueue) {
    let start = std::time::Instant::now();

    // 1. Resolve API key from secret vault (never leaves SpacetimeDB)
    let api_key = ctx.with_tx(|tx| {
        tx.db.secret_vault()
            .filter(|s| s.key_name == format!("{}_api_key", request.provider))
            .next()
            .map(|s| decrypt_aes256gcm(&s.encrypted_value))
    }).flatten();

    let Some(api_key) = api_key else {
        write_error_response(ctx, &request, "no_api_key", "API key not found in vault");
        return;
    };

    // 2. Build the HTTP request based on provider type
    let (url, body, headers) = match request.provider.as_str() {
        "anthropic" => build_anthropic_request(&request, &api_key),
        "minimax" | "together" | "groq" | "openrouter" =>
            build_openai_compat_request(&request, &api_key),
        "ollama" => build_ollama_request(&request),  // No API key needed
        "vllm" => build_vllm_request(&request, &api_key),
        _ => {
            write_error_response(ctx, &request, "unknown_provider",
                &format!("Unknown provider: {}", request.provider));
            return;
        }
    };

    // 3. Make the HTTP call — THIS IS THE KEY CAPABILITY OF PROCEDURES
    let http_result = ctx.http.fetch(&url, HttpMethod::Post, &headers, &body);

    match http_result {
        Ok(response) => {
            let latency_ms = start.elapsed().as_millis() as u64;
            let parsed = parse_inference_response(&request.provider, &response.body);

            // 4. Write response atomically — agent receives via subscription
            ctx.with_tx(|tx| {
                // Write the response
                tx.db.inference_response().insert(InferenceResponse {
                    response_id: 0,
                    request_id: request.request_id,
                    agent_id: request.agent_id.clone(),
                    status: "completed".into(),
                    content_json: parsed.content_json,
                    model_used: parsed.model_used,
                    input_tokens: parsed.input_tokens,
                    output_tokens: parsed.output_tokens,
                    cache_read_tokens: parsed.cache_read_tokens,
                    cache_write_tokens: parsed.cache_write_tokens,
                    latency_ms,
                    cost_usd: compute_cost(&parsed),
                    created_at: timestamp_now(),
                });

                // Update agent budget
                if let Some(mut budget) = tx.db.agent_budget()
                    .filter(|b| b.agent_id == request.agent_id)
                    .next() {
                    budget.used_tokens += parsed.input_tokens + parsed.output_tokens;
                    budget.used_usd += compute_cost(&parsed);
                    tx.db.agent_budget().update(budget);
                }

                // Update provider rate counters
                if let Some(mut provider) = tx.db.inference_provider()
                    .filter(|p| p.provider_id == request.provider)
                    .next() {
                    provider.current_rpm += 1;
                    provider.current_tpm += parsed.input_tokens + parsed.output_tokens;
                    provider.avg_latency_ms = (provider.avg_latency_ms * 9 + latency_ms) / 10;
                    tx.db.inference_provider().update(provider);
                }
            });
        }
        Err(e) => {
            write_error_response(ctx, &request, "http_error", &format!("{e:?}"));
        }
    }
}

// ─── Scheduled Reducer: Rate limit window reset ───────────

#[table(accessor = rate_reset_schedule, scheduled(reset_rate_counters))]
pub struct RateResetSchedule {
    #[primary_key]
    #[auto_inc]
    pub id: u64,
    pub scheduled_at: ScheduleAt,
}

#[reducer]
fn reset_rate_counters(ctx: &ReducerContext, _schedule: RateResetSchedule) {
    // Reset RPM/TPM counters every 60 seconds
    for mut provider in ctx.db.inference_provider().iter() {
        provider.current_rpm = 0;
        provider.current_tpm = 0;
        ctx.db.inference_provider().update(provider);
    }

    // Re-schedule self for next window
    ctx.db.rate_reset_schedule().insert(RateResetSchedule {
        id: 0,
        scheduled_at: ScheduleAt::Interval(Duration::from_secs(60).into()),
    });
}
```

#### Why This Matters for 100s of Agents

Without centralized inference, each of 100 agents independently calls the Anthropic API:

```
BEFORE (decentralized):
Agent-1 ──► Anthropic API ◄── Agent-2     Rate limits hit constantly
Agent-3 ──► Anthropic API ◄── Agent-4     No global budget enforcement
...         (429 errors)      ...          Each agent retries independently
Agent-100 ─► Anthropic API                 Thundering herd on rate recovery
```

With SpacetimeDB procedures as the gateway:

```
AFTER (centralized via SpacetimeDB):
Agent-1 ─┐                                 ┌─► InferenceResponse (subscription)
Agent-2 ─┤  request_inference()             │   Agent-1 gets its response
Agent-3 ─┼──► InferenceQueue ──► Procedure ─┼─► Agent-2 gets its response
...      │    (reducer validates   (ctx.http │   ...
Agent-100┘     budget + rate)      calls API)└─► Agent-100 gets its response
```

**Benefits**:

| Benefit | Mechanism |
|---------|-----------|
| **Global rate limiting** | Reducer checks `current_rpm` before queuing — rejects at the gate |
| **Budget enforcement** | Reducer checks `AgentBudget` — no agent can overspend |
| **API keys never leave SpacetimeDB** | Procedure reads from `secret_vault`, decrypts in-process |
| **Automatic provider failover** | `select_provider()` picks healthiest endpoint; if Anthropic is rate-limited, routes to MiniMax |
| **Complete audit trail** | Every request + response persisted in `InferenceQueue` + `InferenceResponse` |
| **Cost tracking** | `cost_usd` computed per-request, aggregated in `AgentBudget` |
| **Priority queuing** | High-priority requests (domain layer work) processed before low-priority (formatting) |
| **RL feedback loop** | RL engine reads `InferenceResponse` (latency, cost, tokens) to optimize future model selection |
| **Zero agent-side secrets** | Agents never see API keys — they just call a reducer |
| **hex-chat visibility** | Dashboard subscribes to `InferenceResponse` — sees every call in real-time |

#### Agent-Side: Subscription-Based Response Handling

```rust
// hex-agent/adapters/spacetime_inference.rs

/// Inference adapter that routes all LLM calls through SpacetimeDB procedures
pub struct SpacetimeInferenceAdapter {
    stdb: SpacetimeDbConnection,
    agent_id: String,
    /// Populated by subscription to InferenceResponse filtered by agent_id
    pending_responses: Arc<DashMap<u64, oneshot::Sender<InferenceResponse>>>,
}

impl IInferencePort for SpacetimeInferenceAdapter {
    async fn complete(&self, req: InferenceRequest) -> Result<InferenceResponse> {
        // 1. Create a oneshot channel for the response
        let (tx, rx) = oneshot::channel();

        // 2. Call the reducer — queues the request
        let request_id = self.stdb.call_reducer("request_inference", &RequestArgs {
            agent_id: self.agent_id.clone(),
            model: req.model.clone(),
            messages_json: serde_json::to_string(&req.messages)?,
            tools_json: serde_json::to_string(&req.tools)?,
            max_tokens: req.max_tokens,
            temperature: req.temperature,
            thinking_budget: req.thinking_budget,
            cache_control: req.cache_control,
            priority: req.priority,
        }).await?;

        // 3. Register the pending response
        self.pending_responses.insert(request_id, tx);

        // 4. Wait for SpacetimeDB subscription to deliver the response
        //    (no polling — push-based via WebSocket subscription)
        let response = tokio::time::timeout(
            Duration::from_secs(120),  // 2 min timeout for long inference
            rx,
        ).await??;

        Ok(response)
    }

    fn capabilities(&self) -> InferenceCapabilities {
        // Capabilities come from InferenceProvider table subscription
        // — always up-to-date, no HTTP call needed
        self.cached_capabilities.read().clone()
    }
}

// Subscription handler — called by SpacetimeDB SDK when InferenceResponse rows arrive
fn on_inference_response(response: &InferenceResponse, pending: &DashMap<u64, oneshot::Sender<_>>) {
    if let Some((_, tx)) = pending.remove(&response.request_id) {
        let _ = tx.send(response.clone());
    }
}
```

#### Streaming Support

For streaming responses (essential for interactive use), the procedure writes **partial chunks** to a streaming table:

```rust
#[table(public)]
pub struct InferenceStreamChunk {
    #[primary_key]
    #[auto_inc]
    pub chunk_id: u64,
    pub request_id: u64,
    pub agent_id: String,
    pub chunk_type: String,    // "text_delta", "tool_use_start", "input_json_delta", "message_stop"
    pub content: String,
    pub sequence: u32,         // Ordering within the stream
    pub created_at: String,
}

// In the procedure, for streaming providers:
#[spacetimedb::procedure]
fn execute_inference_streaming(ctx: &mut ProcedureContext, request: InferenceQueue) {
    // SSE streaming from Anthropic → parse each event → write chunk row
    // Agent subscribes to InferenceStreamChunk filtered by request_id
    // Chunks arrive via SpacetimeDB subscription in real-time
    let stream = ctx.http.fetch_stream(&url, HttpMethod::Post, &headers, &body);

    let mut sequence = 0u32;
    for event in stream {
        let chunk = parse_sse_event(&event);
        ctx.with_tx(|tx| {
            tx.db.inference_stream_chunk().insert(InferenceStreamChunk {
                chunk_id: 0,
                request_id: request.request_id,
                agent_id: request.agent_id.clone(),
                chunk_type: chunk.chunk_type,
                content: chunk.content,
                sequence,
                created_at: timestamp_now(),
            });
        });
        sequence += 1;
    }
}
```

This gives agents **real-time streaming through SpacetimeDB subscriptions** — the developer in hex-chat can watch tokens appear live across all agents simultaneously.

### Migration Plan

| Phase | Deliverable | Crates Affected | Risk |
|-------|------------|-----------------|------|
| **0** | Extract `hex-core` with shared types + ports | New crate | Low — additive |
| **1a** | SpacetimeDB modules: `file-lock-manager`, `architecture-enforcer`, `conflict-resolver` | spacetime-modules | Low — new modules |
| **1b** | SpacetimeDB module: `inference-gateway` with procedures for HTTP calls | spacetime-modules | Medium — procedures are beta |
| **2** | `hex-agent` depends on `hex-core`, pre-write validation in code_writer | hex-agent, hex-core | Medium — changes write path |
| **3** | `hex-nexus` depends on `hex-core`, remove duplicate types | hex-nexus, hex-core | Medium — refactor |
| **4** | `hex-chat` TUI binary with SpacetimeDB subscriptions | New crate | Low — additive |
| **5** | `hex-cli` Rust binary, MCP server in Rust | New crate | High — replaces TS CLI |
| **6** | Direct SpacetimeDB SDK in hex-agent (bypass HTTP) | hex-agent | Medium — new adapter |
| **7** | Retire TS `src/` directory | TS removal | High — breaking change |

### What This Changes for Each Component

| Component | Before | After |
|-----------|--------|-------|
| **hex-nexus** | Monolithic hub: state, coordination, analysis, agents, chat, fleet | Focused orchestrator: SpacetimeDB adapter, agent lifecycle, HTTP API for external clients |
| **hex-agent** | Reimplements Claude Code loop, HTTP-polls hub | Autonomous worker with SpacetimeDB subscriptions, pre-write validation, pluggable inference |
| **hex-chat** | `chat.html` debug tool | Standalone TUI + Web dashboard, CEO command center |
| **hex-cli** | 300-line TS composition root, MCP server | Thin Rust binary, delegates to hex-nexus API |
| **SpacetimeDB** | Storage backend | Active coordination plane + inference gateway — enforcement reducers, procedures make LLM calls, API keys never leave DB |
| **Claude Code** | The runtime that hosts hex | One of many pluggable inference adapters |
| **RL Engine** | HTTP API in hex-nexus | SpacetimeDB-native, agents subscribe to Q-values |

## Consequences

### Positive

- **Single language**: Rust everywhere eliminates TS↔Rust impedance mismatch (build hash verification, lock file polling, child process management)
- **Real-time coordination**: SpacetimeDB subscriptions replace polling — 100s of agents get instant updates
- **Pre-write enforcement**: Boundary violations become impossible, not merely detectable
- **Centralized inference gateway**: All LLM calls flow through SpacetimeDB procedures — global rate limiting, budget enforcement, API keys never leave the database, complete audit trail
- **Pluggable inference**: Any LLM backend works — from Claude Opus to local Ollama — RL engine optimizes selection
- **Developer sovereignty**: hex-chat gives the developer CEO-level control without needing to understand agent internals
- **Reduced binary count**: 3 binaries (hex-nexus, hex-agent, hex-chat) + 1 thin CLI, down from 3 binaries + Node.js runtime
- **Compile-time safety**: Rust's type system catches integration errors that TS generics miss
- **File conflict prevention**: Distributed locking prevents the merge races that plague multi-agent codegen

### Negative

- **Migration cost**: Retiring the TS CLI is a breaking change for existing users — need npm shim period
- **SpacetimeDB coupling**: More functionality in SpacetimeDB means more dependency on its stability and SDK maturity
- **Learning curve**: Contributors must know Rust — no more "easy" TS contributions
- **hex-chat complexity**: Building a good TUI is non-trivial — `ratatui` has ergonomic rough edges
- **Inference latency overhead**: Routing through SpacetimeDB adds ~5-15ms per inference call vs. direct HTTP
- **Procedure maturity**: SpacetimeDB procedures are currently in beta — API may change
- **Streaming complexity**: SSE streaming through SpacetimeDB subscription adds implementation complexity vs. direct streaming
- **Testing overhead**: Pre-write validation adds latency to every file write (~10-50ms per SpacetimeDB roundtrip)

### Mitigations

| Risk | Mitigation |
|------|-----------|
| TS CLI breakage | npm package downloads Rust binary (like esbuild/turbo pattern) — same `hex` command, different runtime |
| SpacetimeDB outage | Fallback to in-memory coordination (current behavior) — degrade gracefully |
| Inference latency | ~10ms overhead is negligible vs. 2-30s inference time; eliminated thundering herd saves more |
| Procedure beta | Abstract behind `IInferencePort` — can swap to direct HTTP adapter if procedures regress |
| Streaming via STDB | `InferenceStreamChunk` table with sequence ordering; fallback to direct SSE for interactive CLI |
| Write latency | Batch validation — validate all imports in a single reducer call, not per-import |
| TUI complexity | Start with web dashboard (HTMX), TUI as progressive enhancement |
| Contributor barrier | hex-core domain types are simple Rust structs — low barrier for domain contributions |

## References

- [SpacetimeDB Functions](https://spacetimedb.com/docs/functions) — Reducer architecture for enforcement
- ADR-024: hex-nexus as autonomous hub
- ADR-025: SpacetimeDB state backend
- ADR-026: SpacetimeDB secret broker
- ADR-027: HexFlo native coordination
- ADR-032: Deprecate hex-hub (completed)
- ADR-034: Migrate analyzer to Rust

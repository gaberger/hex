# ADR-2604261303: IModelProvider port + crate split (client / orchestrator / worker)

**Status:** Accepted
**Date:** 2026-04-26
**Accepted:** 2026-04-26
**Drivers:** hex has drifted from its founding goals (model tiering & independence, multi-host scaleout, hexagonal rigor). Symptoms: 50+ files in `hex-cli/src/commands/`, 7 SpacetimeDB modules, `TaskTier` strings threaded through 4 layers, `inference_router` parsing `"model:X|context:Y"` compound action strings, no actual cross-host scheduler, hexagonal rules enforced inside `src/core/` but violated at the workspace level (every crate pulls `tokio`/`reqwest`/`serde_json`).
**Supersedes:** Partially supersedes ADR-2604120202 (tier-aware routing) and ADR-2604241820 (RL-aware model selection) — both retained as policies *over* the new port, not as a replacement for it.

## Context

### Founding goals vs. reality

| Goal | Reality (2026-04-26) |
|------|---------------------|
| Model tiering & independence | `TaskTier` is a string enum hardcoded in 4 places; tier→model is a TOML HashMap; provider switch requires editing the router |
| Multi-host scaleout | Heartbeats + remote-agent registration exist, but no placement policy. `task_assign` and `agent_register` don't talk to each other. Multi-host = a dashboard, not a scheduler |
| Hexagonal rigor | Enforced inside `src/core/` (TS) and partially in `hex-nexus/src/`; not enforced workspace-wide. `hex-cli` depends on `tokio`, `reqwest`, `serde_json`, STDB schemas, HTTP routes, env vars, and filesystem — it is a god-crate |

### Where the complexity actually lives

1. **`hex-cli` is the god-crate.** Daemon, brain, sched, classifier, ADR engine, hook handlers, plan executor, inbox, doctor, dashboard — all in one binary. Adding a feature = adding a `commands/` file.
2. **SpacetimeDB sprawl.** 7 modules: `hexflo-coordination`, `agent-registry`, `inference-gateway`, `secret-grant`, `rl-engine`, `chat-relay`, `neural-lab`. Coordination state is mixed with domain state is mixed with secrets is mixed with experimental ML.
3. **No first-class provider abstraction.** Each new model family (Anthropic, OpenAI, Ollama, llama.cpp, OpenRouter, vLLM) gets bespoke wiring. The router does string-parsing to decide what to call. Capabilities (context window, cost, latency, modality, locality) are implicit, not declared.
4. **CLI is a client of itself.** `hex sched daemon` writes to STDB *via* HTTP to nexus. The CLI both *is* the system and *talks to* the system, with no clear ownership of facts.
5. **Workplan executor is imperative.** 1000+ lines of inline dispatch with timeouts bolted on. The "auto-marks done without dispatching agents" bug in the boot banner is a symptom: there's no single source of truth for "did this task actually run."

### Forces

- **Backward compatibility:** 200+ ADRs, 55 active workplans, multiple downstream targets (`examples/`) consume the current shape. Big-bang rewrite is unacceptable.
- **Single-developer cadence:** redesign must ship in slices that each leave the system working.
- **Existing investments worth preserving:** tree-sitter analysis, hexagonal boundary checker, hexflo coordination *protocol* (the data model, not the WASM implementation), ADR/workplan format, behavioral specs.

### Alternatives considered

| Alternative | Rejected because |
|-------------|------------------|
| Big-bang rewrite | Loses 6 months of accreted knowledge in ADRs, hooks, skills, fixtures |
| "Just add another adapter" | Treats the symptom — every new feature still lands in `hex-cli/src/commands/` |
| Replace STDB entirely | STDB's reactive subscription model is the one thing genuinely earning its keep for the dashboard |
| Microservice explosion | Operational cost outweighs the win for a system that mostly runs on one host today |

## Decision

We will refactor hex into **three layered planes** with **one new core port** (`IModelProvider`). The change ships in 6 phases; each phase leaves the workspace green and `hex --help` working.

### 1. Three planes

```
┌─────────────────────────────────────────────────────────┐
│  Plane A — CONTROL                                      │
│  hex-cli (thin client, ~10 verbs, only clap + reqwest)  │
│  hex-dashboard (read-only, websocket subscriber)        │
└──────────────────────────┬──────────────────────────────┘
                           │ HTTP / websocket
┌──────────────────────────▼──────────────────────────────┐
│  Plane B — ORCHESTRATION                                │
│  hex-orchestrator (renamed nexus, smaller scope)        │
│  Owns: workplan executor, task placement, routing       │
│        policy, reward bookkeeping, artifact store       │
└──────────┬──────────────────────────────────┬──────────┘
           │ ICoordinationStore               │ IModelProvider
           │ (STDB or SQLite)                 │ (per-host)
┌──────────▼─────────────┐         ┌──────────▼──────────┐
│  Plane B' — STATE      │         │  Plane C — EXECUTE  │
│  hexflo-coord (1 STDB  │         │  hex-worker         │
│  module, not 7)        │         │  (slim daemon, no   │
│                        │         │   business logic)   │
└────────────────────────┘         └─────────────────────┘
```

**Plane A — Control.** `hex-cli` is a thin client. ~10 verbs: `submit`, `status`, `cancel`, `logs`, `models`, `hosts`, `plan`, `analyze`, `init`, `doctor`. Everything else moves to orchestrator (sched/brain/daemon/inbox/etc.) or dies. Dashboard becomes read-only.

**Plane B — Orchestration.** One service: `hex-orchestrator` (rename of nexus). Owns workplan execution, task placement, model routing, reward bookkeeping, artifact storage. **Depends only on ports defined in `hex-core`.**

**Plane C — Execution.** New crate: `hex-worker`. A slim daemon you run on every host. Heartbeats to orchestrator, pulls tasks, runs them through local providers, reports results. **No business logic.** No knowledge of workplans/RL/ADRs. Capacity is declared by the worker, not configured centrally. Adding a host = `hex worker register --orchestrator <url>`.

### 2. `IModelProvider` port — the central new abstraction

Defined in `hex-core`. Replaces `TaskTier` as the primary concept on the request path.

```rust
// hex-core/src/ports/model_provider.rs
#[async_trait]
pub trait IModelProvider: Send + Sync {
    /// Static capabilities — declared by the provider, used by routing policy.
    fn capabilities(&self) -> ProviderCapabilities;

    /// Live health — last-known latency, error rate, queue depth.
    async fn health(&self) -> ProviderHealth;

    /// Execute a completion. Pure: input + capabilities → output.
    async fn complete(&self, req: CompletionRequest) -> Result<CompletionResponse, ProviderError>;
}

pub struct ProviderCapabilities {
    pub id: ProviderId,                  // e.g. "ollama:qwen2.5-coder:32b"
    pub family: ModelFamily,             // Qwen, Llama, Claude, GPT, ...
    pub context_window: usize,
    pub max_output_tokens: usize,
    pub modality: Modality,              // Text, Vision, Audio, ...
    pub locality: Locality,              // LocalGPU, LocalCPU, RemoteAPI
    pub cost_per_1k_input: Option<f64>,  // None = free / local
    pub cost_per_1k_output: Option<f64>,
    pub supports_tools: bool,
    pub supports_grammar: bool,
}

pub struct Requirements {
    pub min_context: usize,
    pub max_cost_usd: Option<f64>,
    pub require_local: bool,
    pub require_modality: Modality,
    pub require_tools: bool,
}
```

**Routing becomes a pure function:**

```rust
// hex-orchestrator/src/routing.rs
pub fn route(
    req: &Requirements,
    providers: &[ProviderCapabilities],
    history: &RoutingHistory,        // for RL / cost tracking / latency
    policy: &dyn RoutingPolicy,
) -> Result<ProviderId, RoutingError>;
```

**Tier names become presets over `Requirements`,** not a separate type:

```rust
pub const T1_PRESET: Requirements = Requirements { min_context: 4096,  require_local: true,  ..};
pub const T2_PRESET: Requirements = Requirements { min_context: 16384, require_local: true,  ..};
pub const T2_5_PRESET: Requirements = Requirements { min_context: 32768, require_local: true, ..};
pub const T3_PRESET: Requirements = Requirements { min_context: 200000, require_local: false, ..};
```

**RL becomes one `RoutingPolicy` impl,** not a hardcoded scoring loop in the router. Q-table moves from a STDB module to orchestrator-internal state.

### 3. Workspace dependency rules (enforced)

Add `xtask check-boundaries` that runs on CI:

| Crate | May depend on | May NOT depend on |
|-------|---------------|-------------------|
| `hex-core` | `serde`, `thiserror`, `uuid`, `async-trait` | anything else |
| `hex-cli` | `hex-core`, `clap`, `reqwest` | `tokio`-runtime details, STDB, filesystem APIs |
| `hex-orchestrator` | `hex-core`, runtime adapters | `hex-cli`, `hex-worker`, `hex-dashboard-assets` |
| `hex-worker` | `hex-core`, `hex-providers-*` | orchestrator internals, workplan format |
| `hex-providers-ollama` | `hex-core`, `reqwest` | other provider crates |
| `hex-providers-anthropic` | `hex-core`, `anthropic SDK` | other provider crates |

The "adapters never import other adapters" rule that already governs `src/core/` is promoted to a **workspace-level** invariant.

### 4. SpacetimeDB consolidation

Two modules survive:
- `hexflo-coordination` — agents, tasks, heartbeats, swarm state. Pure coordination.
- `artifacts` (new, optional) — workplans + ADRs as queryable rows for the dashboard. Filesystem remains source of truth; STDB is a projected index.

Deleted:
- `rl-engine` → orchestrator-internal state
- `secret-grant` → OS keyring + env
- `chat-relay` → log file + dashboard tail
- `neural-lab` → moved to a sandbox repo outside `hex-intf`
- `inference-gateway`, `agent-registry` → folded into orchestrator HTTP API; STDB only stores the *resulting* assignments

### 5. Workplan executor as a state machine

Replace the imperative dispatch loop with:

```rust
enum TaskState { Pending, Claimed { worker, at }, Running { worker, started }, Done { artifact }, Failed { reason } }

trait TaskTransitions {
    fn claim(&self, t: &Task, w: WorkerId) -> Result<TaskState, TransitionError>;
    fn start(&self, t: &Task) -> Result<TaskState, TransitionError>;
    fn complete(&self, t: &Task, artifact: Artifact) -> Result<TaskState, TransitionError>;
    fn fail(&self, t: &Task, reason: String) -> Result<TaskState, TransitionError>;
}
```

Pure transitions; persistence is one `IStateStore` adapter. The "auto-marks done without dispatch" bug becomes impossible because `Done` requires an `Artifact`.

## Consequences

**Positive:**
- Adding a new model = one file (`hex-providers-foo` crate implementing `IModelProvider`). Today it requires touching the router, the tier config, the workplan schema, and the dashboard.
- Multi-host becomes real: workers declare capacity, orchestrator places work, no central config of which host has which model.
- `hex-cli` shrinks from ~50 command files to ~10. New features go where they belong instead of the god-crate.
- Hexagonal rules enforced at the *workspace* level, not just inside one crate.
- The "did this task actually run" question has a single answer: read the `TaskState` machine.
- RL becomes testable in isolation (it's a `RoutingPolicy` over a pure function, not coupled to inference plumbing).
- STDB module count: 7 → 1 (or 2 with optional `artifacts`). Operational surface area drops accordingly.

**Negative:**
- Six-phase migration over multiple weeks. Each phase must keep the system working.
- Some current behaviors (e.g., chat-relay dashboard tab, neural-lab) lose their STDB-backed implementation and must move or die.
- `hex-cli` users will see commands relocate (e.g., `hex sched daemon` becomes `hex-orchestrator daemon`). Wrappers can preserve the old surface during transition.
- `hex-worker` is a new long-running process to operate. Mitigated by making it the *only* per-host daemon (replaces the current implicit "CLI doubles as daemon" mode).

**Mitigations:**
- Each phase ends with a green `cargo build -p hex-cli -p hex-orchestrator -p hex-worker --release` and a passing `bun test` / `cargo test --workspace`.
- Old commands keep working via thin shims that delegate to the new orchestrator endpoints.
- Deleted STDB modules get a final snapshot exported to JSON before deletion, archived in `docs/migrations/`.
- ADR-2604120202 (tier-aware routing) is **retained** as the T1/T2/T2.5/T3 preset definitions over `Requirements`. ADR-2604241820 (RL-aware selection) is **retained** as the default `RoutingPolicy` impl. Neither is rolled back; both are reframed.

## Implementation

| Phase | Description | Exit criteria | Status |
|-------|------------|---------------|--------|
| **P1** | Define `IModelProvider` + `Requirements` + `RoutingPolicy` in `hex-core`. Port the existing Ollama, Anthropic, OpenRouter adapters into a new `hex-providers/` crate family. **Do not** rewire the router yet. | New crates compile. Unit tests for one provider pass. | Pending |
| **P2** | Rewire `inference_router` to call `route(req, providers, history, policy)` instead of doing inline tier lookup + string parsing. `TaskTier` enum becomes a thin wrapper around `Requirements` presets. Existing behavior preserved. | `inference_router/mod.rs` < 200 lines. All existing inference tests green. | Pending |
| **P3** | Extract `hex-orchestrator` crate (rename from `hex-nexus`). Move `sched`, `brain`, `daemon`, `inbox`, `workplan_executor`, `coordination` into it. `hex-cli` keeps only commands that talk to the orchestrator over HTTP. | `cargo build -p hex-cli` does not pull in `tokio`-runtime, `axum`, or STDB SDK. | Pending |
| **P4** | Introduce `hex-worker` crate. Workers register with orchestrator via `POST /api/workers`, declare capacity + providers, heartbeat, pull tasks via long-poll or websocket. Orchestrator's placement policy becomes a pure function. Single-host setup runs orchestrator + one worker on the same machine (compatible with today). | `hex worker register` works; orchestrator can route a task to a remote worker. | Pending |
| **P5** | STDB consolidation: migrate RL state into orchestrator-internal storage; delete `rl-engine`, `secret-grant`, `chat-relay`, `neural-lab` modules; archive their data. Collapse `agent-registry` + `inference-gateway` into `hexflo-coordination`. | Module count: 7 → 2. Dashboard still works. | Pending |
| **P6** | Workplan executor refactored into the `TaskState` state machine. Add `xtask check-boundaries` enforcing workspace dependency rules. Wire into CI. | Boundary check passes; "auto-marks done without dispatch" bug is structurally impossible. | Pending |

Each phase is independently revertable and produces a working system. Order is not negotiable: P1 unblocks P2; P3 unblocks P4; P4 unblocks P5; P6 stands alone but is most useful after P3.

## References

- ADR-2604120202 — Tier-aware routing (retained as preset definitions over `Requirements`)
- ADR-2604241820 — RL-aware model selection (retained as default `RoutingPolicy` impl)
- ADR-2604050900 — 7-module SpacetimeDB layout (this ADR proposes consolidating to 1–2)
- ADR-027 — HexFlo native Rust coordination (the *protocol* survives; the WASM module count shrinks)
- CLAUDE.md "Hexagonal Architecture Rules (ENFORCED)" — promoted from intra-crate to workspace-wide
- Boot banner symptom: "hex plan execute auto-marks tasks as 'done' without dispatching agents via MCP. All 55 stalled workplans show status=Failed" — addressed structurally by P6

# Ports Sigma-Algebra -- hex Effect Signature

**Phase:** P1 of [ADR-2604111229](../adrs/ADR-2604111229-algebraic-formalization-of-process-flow.md)
**Source of truth:** `hex-core/src/ports/*.rs`
**Last verified against source:** 2026-04-12

---

## Overview

hex's 10 port traits form an **algebraic signature** Sigma. Each port is a
sort in the signature; each trait method is an operation symbol with a typed
arity. A program in hex layer L is a term in the free algebra `T(Sigma_L)` --
it can only reference operations from the sub-signature visible to its layer.

The composition root (`hex-nexus`, `hex-cli`) is the unique Sigma-algebra
morphism `interpret: T(Sigma) -> IO` -- the only place where abstract port
operations become real side effects. Every adapter is a partial interpretation
of this morphism: `FileSystemAdapter` interprets `Sigma_fs`, `OllamaInferenceAdapter`
interprets `Sigma_inf`, and so on.

This document enumerates all 10 ports, 43 operations, and their type
signatures, then defines the layer visibility rules as sub-signature
inclusions.

---

## Notation

```
op: A x B -> Result<C, E>      -- operation taking A and B, returning C or error E
op: A -> C                     -- infallible operation (no error case)
&self / &mut self omitted       -- all operations are method calls on the port
async marked where applicable   -- async ops return Future<Result<C, E>>
```

The error type for each port is its **bottom element** -- the absorbing
element of monadic composition over `Result`. When an operation returns
`Err(e)`, all downstream Kleisli-composed operations short-circuit.

---

## Sigma_fs -- IFileSystemPort

**Source:** `hex-core/src/ports/file_system.rs`
**Effect class:** Disk I/O (read/write), path resolution
**Error type (bottom):** `FileSystemError`

| # | Operation | Signature | Effect |
|---|-----------|-----------|--------|
| 1 | `read_file` | `path: &str -> Result<String, FileSystemError>` | Read |
| 2 | `write_file` | `path: &str x content: &str -> Result<(), FileSystemError>` | Write |
| 3 | `file_exists` | `path: &str -> Result<bool, FileSystemError>` | Read |
| 4 | `list_directory` | `path: &str -> Result<Vec<String>, FileSystemError>` | Read |
| 5 | `glob` | `pattern: &str x base: &str -> Result<Vec<String>, FileSystemError>` | Read |

**Error variants:** `PathTraversal`, `NotFound`, `PermissionDenied`, `Io`

**Invariants:**
- `read_file(p)` after `write_file(p, c)` yields `Ok(c)` (write-read coherence)
- `file_exists(p)` returns `Ok(true)` iff `read_file(p)` returns `Ok(_)` (existence coherence)
- `PathTraversal` is returned for any `p` that escapes the sandbox root (security invariant)

---

## Sigma_inf -- IInferencePort

**Source:** `hex-core/src/ports/inference.rs`
**Effect class:** Network I/O (LLM API calls), token consumption
**Error type (bottom):** `InferenceError`

| # | Operation | Signature | Effect |
|---|-----------|-----------|--------|
| 1 | `complete` | `InferenceRequest -> Result<InferenceResponse, InferenceError>` | Network, tokens |
| 2 | `stream` | `InferenceRequest -> Result<Stream<StreamChunk>, InferenceError>` | Network, tokens, streaming |
| 3 | `health` | `() -> Result<HealthStatus, InferenceError>` | Network (probe) |
| 4 | `capabilities` | `() -> InferenceCapabilities` | Pure (cached metadata) |

**Error variants:** `RateLimited`, `BudgetExceeded`, `ProviderUnavailable`, `ApiError`, `Network`, `UnknownProvider`

**Key types:**
- `InferenceRequest` carries: model, system_prompt, messages, tools, max_tokens, temperature, thinking_budget, cache_control, priority, grammar (GBNF)
- `InferenceResponse` carries: content blocks, model_used, stop_reason, token counts (input/output/cache_read/cache_write), latency_ms
- `HealthStatus`: `Ok { models }` | `Degraded { reason }` | `Unreachable { reason }`
- `Priority`: Low(0), Normal(1), High(2), Critical(3)
- `ModelTier`: Opus, Sonnet, Haiku, Local

**Invariants:**
- `complete(r)` and `stream(r)` are semantically equivalent -- same final content, different delivery
- `capabilities()` is pure and idempotent (no side effects)
- `health()` is a probe -- must not consume tokens or alter state
- `BudgetExceeded` is returned when cumulative token usage exceeds the agent's allocation

---

## Sigma_coord -- ICoordinationPort

**Source:** `hex-core/src/ports/coordination.rs`
**Effect class:** Distributed state (SpacetimeDB), file locking, swarm lifecycle
**Error type (bottom):** `CoordinationError`

| # | Operation | Signature | Effect |
|---|-----------|-----------|--------|
| 1 | `acquire_file_lock` | `file_path x agent_id x LockType -> Result<FileLock, CoordinationError>` | Lock acquire |
| 2 | `release_file_lock` | `file_path x agent_id -> Result<(), CoordinationError>` | Lock release |
| 3 | `validate_write` | `agent_id x file_path x imports: &[String] -> Result<WriteValidation, CoordinationError>` | Boundary check |
| 4 | `swarm_init` | `name x topology -> Result<SwarmInfo, CoordinationError>` | State create |
| 5 | `swarm_status` | `() -> Result<Vec<SwarmInfo>, CoordinationError>` | State read |
| 6 | `task_create` | `swarm_id x title -> Result<SwarmTask, CoordinationError>` | State create |
| 7 | `task_complete` | `task_id x result -> Result<(), CoordinationError>` | State transition |
| 8 | `memory_store` | `key x value x scope? -> Result<(), CoordinationError>` | KV write |
| 9 | `memory_retrieve` | `key -> Result<Option<String>, CoordinationError>` | KV read |
| 10 | `memory_search` | `query -> Result<Vec<(String, String)>, CoordinationError>` | KV search |
| 11 | `heartbeat` | `agent_id x AgentStatus x turn_count x token_usage -> Result<(), CoordinationError>` | Liveness signal |

**Error variants:** `LockConflict { file_path, held_by }`, `BoundaryViolation`, `SwarmNotFound`, `TaskNotFound`, `Connection`

**Invariants:**
- `acquire_file_lock(p, a, Exclusive)` fails with `LockConflict` if any other agent holds a lock on `p` (mutual exclusion)
- `acquire_file_lock(p, a, SharedRead)` succeeds iff no `Exclusive` lock exists on `p` (reader-writer protocol)
- `release_file_lock(p, a)` is idempotent -- releasing an unheld lock is a no-op success
- `task_complete(id, _)` is a terminal transition -- a completed task cannot be re-opened (monotonic state)
- `memory_store(k, v, _)` followed by `memory_retrieve(k)` yields `Ok(Some(v))` (KV coherence)
- `heartbeat` must be called within 45s intervals to avoid `stale` status, 120s to avoid `dead` + task reclamation

---

## Sigma_enf -- IEnforcementPort

**Source:** `hex-core/src/ports/enforcement.rs`
**Effect class:** Pure (decision function, no I/O)
**Error type (bottom):** None (infallible)

| # | Operation | Signature | Effect |
|---|-----------|-----------|--------|
| 1 | `check` | `&EnforcementContext -> EnforcementResult` | Pure |

**Key types:**
- `EnforcementContext` carries: agent_id, workplan_id, swarm_id, task_id, target_file, operation, is_background
- `EnforcementResult`: `Allow` | `Warn(String)` | `Block(String)`
- `EnforcementMode`: `Mandatory` (blocks) | `Advisory` (warns) | `Disabled`

**Invariants:**
- `check` is a pure function -- same context always yields the same result (referential transparency)
- `check` is total -- every `EnforcementContext` produces a result (no panics, no errors)
- In `Mandatory` mode, a `Block` result MUST prevent the downstream operation from executing
- In `Advisory` mode, a `Block` result is downgraded to `Warn`

The simplest port in the signature -- a single pure guard function. Its algebraic role: it sits in front of effectful operations and gates them. In Kleisli terms, it converts `Block` into short-circuit before the effectful operation fires.

---

## Sigma_secret -- ISecretPort

**Source:** `hex-core/src/ports/secret.rs`
**Effect class:** Vault I/O (read/write encrypted secrets)
**Error type (bottom):** `SecretError`

| # | Operation | Signature | Effect |
|---|-----------|-----------|--------|
| 1 | `resolve_secret` | `key: &str -> Result<String, SecretError>` | Vault read |
| 2 | `claim_secrets` | `agent_id: &str -> Result<ClaimResult, SecretError>` | Vault read + consume |
| 3 | `grant_secret` | `&SecretGrant -> Result<(), SecretError>` | Vault write |
| 4 | `revoke_secret` | `agent_id x key -> Result<(), SecretError>` | Vault delete |

**Error variants:** `NotFound`, `Expired { agent_id, key }`, `AlreadyClaimed`, `VaultUnavailable`, `DecryptionFailed`

**Invariants:**
- `claim_secrets(a)` is **one-shot** -- calling it twice for the same agent returns `AlreadyClaimed` (linear consumption)
- `grant_secret(g)` followed by `resolve_secret(g.key)` yields `Ok(value)` (grant-resolve coherence)
- `revoke_secret(a, k)` followed by `resolve_secret(k)` for agent `a` yields `Err(NotFound)` (revocation completeness)
- Grants carry a TTL -- `resolve_secret` returns `Err(Expired)` after TTL elapses (temporal linearity)

**Algebraic note:** The one-shot claim semantics make this port the closest to **linear logic** in the signature. A secret grant is a linear resource: it can be consumed exactly once. This is the foundation for P5 (effect row types for capabilities).

---

## Sigma_brain -- IBrainPort

**Source:** `hex-core/src/ports/brain.rs`
**Effect class:** Pure computation + mutable state (Q-learning table)
**Error type (bottom):** `Box<dyn Error>` (only on `probe_capabilities`)

| # | Operation | Signature | Effect |
|---|-----------|-----------|--------|
| 1 | `parse_intent` | `request: &str -> Intent` | Pure |
| 2 | `probe_capabilities` | `() -> Result<BrainCapabilities, Box<dyn Error>>` | Read (system probe) |
| 3 | `route_request` | `&Intent x &BrainCapabilities -> RoutingDecision` | Pure |
| 4 | `record_outcome` | `method x intent_type x Outcome x latency_ms -> ()` | Mutable state (Q-table update) |
| 5 | `get_scores` | `() -> Vec<MethodScore>` | Read (Q-table) |
| 6 | `get_best_method` | `intent_type: &str -> Option<String>` | Read (Q-table) |

**Invariants:**
- `parse_intent` is pure -- same input always produces the same `Intent`
- `route_request` is pure given the same intent and capabilities
- `record_outcome` is the only mutating operation -- it updates the Q-learning table
- After sufficient `record_outcome` calls, `get_best_method` converges to the empirically optimal method (RL convergence)
- `probe_capabilities` is fallible because it may query system state (Ollama running? GPU available?)

**Algebraic note:** This port mixes pure decision functions with a stateful learning loop. The Q-table acts as an **accumulator monoid**: outcomes are `mappend`'d, `get_best_method` is the `mconcat` readback. The learning dynamics live inside the adapter, not the port signature.

---

## Sigma_build -- IBuildPort

**Source:** `hex-core/src/ports/build.rs`
**Effect class:** Process execution (subprocess spawning)
**Error type (bottom):** None (errors encoded in `BuildOutput.success`)

| # | Operation | Signature | Effect |
|---|-----------|-----------|--------|
| 1 | `detect_toolchain` | `project_dir: &str -> Option<BuildToolchain>` | Filesystem probe |
| 2 | `compile` | `project_dir: &str -> BuildOutput` | Subprocess |
| 3 | `lint` | `project_dir: &str -> BuildOutput` | Subprocess |
| 4 | `test` | `project_dir: &str -> BuildOutput` | Subprocess |

**Key types:**
- `BuildToolchain`: language, compile_cmd, lint_cmd, test_cmd
- `BuildOutput`: success, exit_code, stdout, stderr, diagnostics
- `Diagnostic`: file, line, column, severity, message

**Invariants:**
- `compile(d).success == true` is a **precondition** for `lint(d)` and `test(d)` in the standard pipeline
- Operations are idempotent (assuming no source changes between calls)
- The pipeline ordering (compile -> lint -> test) is a BLOCKING gate sequence

**Algebraic note:** This port encodes failures in the return value rather than `Result`. Every call terminates with a value (the port is **total**). The gate ordering maps to Petri net transitions: each deposits a `success` token that enables the next.

---

## Sigma_sandbox -- ISandboxPort

**Source:** `hex-core/src/ports/sandbox.rs`
**Effect class:** Container lifecycle (Docker/microVM)
**Error type (bottom):** `SandboxError`

| # | Operation | Signature | Effect |
|---|-----------|-----------|--------|
| 1 | `spawn` | `SandboxConfig -> Result<SpawnResult, SandboxError>` | Container create + start |
| 2 | `stop` | `container_id: &str -> Result<(), SandboxError>` | Container stop + remove |
| 3 | `status` | `container_id: &str -> Result<String, SandboxError>` | Container inspect |
| 4 | `list` | `() -> Result<Vec<SpawnResult>, SandboxError>` | Container enumerate |

**Invariants:**
- `spawn(c)` returns a unique `container_id` -- no two spawns return the same id
- `stop(id)` is idempotent -- stopping an already-stopped container is a no-op success
- Lifecycle FSM: `spawned -> running -> stopped` (3 places, 2 transitions)

---

## Sigma_agent -- IAgentRuntimePort

**Source:** `hex-core/src/ports/agent_runtime.rs`
**Effect class:** Task dispatch (inter-process communication)
**Error type (bottom):** `SandboxError`

| # | Operation | Signature | Effect |
|---|-----------|-----------|--------|
| 1 | `execute_task` | `AgentTask -> Result<ToolResult, SandboxError>` | Dispatch + await |
| 2 | `report_completion` | `task_id x result -> Result<(), SandboxError>` | State transition |

**Invariants:**
- `execute_task` is blocking -- dispatches and waits for agent completion
- `execute_task(t)` implies the agent has required capabilities for task `t` (capability precondition -- cross-port dependency with `Sigma_secret`)

---

## Sigma_ctx -- IContextCompressorPort

**Source:** `hex-core/src/ports/context_compressor.rs`
**Effect class:** Pure computation (text transformation)
**Error type (bottom):** None (infallible)

| # | Operation | Signature | Effect |
|---|-----------|-----------|--------|
| 1 | `compress_tool_output` | `output: &str x budget_tokens: u32 -> String` | Pure |
| 2 | `estimate_tokens` | `text: &str -> u32` | Pure (default: len/4) |

**Invariants:**
- Returns input unchanged if already within budget (identity below budget)
- Output always fits within budget (budget guarantee)
- Code blocks and error lines preserved verbatim (lossless for structured content)
- **Contraction mapping** on the string monoid: output length <= input length

---

## Layer Visibility (Sub-Signature Inclusions)

The hexagonal boundary rules as sub-signature access control:

```
Layer               Visible sub-signatures
────────────────────────────────────────────────────────────
domain/             (empty) -- defines value types, no port calls
ports/              Sigma_domain (value types only, no operations)
usecases/           Sigma (all ports, via trait objects)
adapters/primary/   Sigma_L for the port L they implement
adapters/secondary/ Sigma_L for the port L they implement
composition-root    Sigma (full signature -- wires everything)
```

### Formal Statement

Let `Sigma = Sigma_fs + Sigma_inf + Sigma_coord + Sigma_enf + Sigma_secret + Sigma_brain + Sigma_build + Sigma_sandbox + Sigma_agent + Sigma_ctx` be the coproduct of all port signatures.

**Theorem (Hexagonal Containment):** A well-formed hex program in layer `L` is a term `t in T(Sigma_L)`. If `t` contains an operation `op in Sigma_P` where `P` is not in `visible(L)`, then `t` violates the hexagonal boundary.

**Current enforcement:** `hex analyze` checks import edges. It does NOT check operation-level invocations. The Sigma-algebra is strictly stronger.

---

## The Free Algebra View

A program in hex composes port operations without choosing implementations.
The program is a **term** in `T(Sigma)`:

```
let config = fs.read_file("config.json")?;        -- Sigma_fs
let intent = brain.parse_intent(&config);           -- Sigma_brain
let decision = brain.route_request(&intent, &caps); -- Sigma_brain
let response = inference.complete(request)?;         -- Sigma_inf
coord.task_complete(task_id, &summary)?;             -- Sigma_coord
```

This term lives in `T(Sigma_fs + Sigma_brain + Sigma_inf + Sigma_coord)`.
Legal in `usecases/`, illegal in `adapters/secondary/fs/`.

The **composition root** provides the **interpretation morphism**:

```
interpret: T(Sigma) -> IO

interpret(fs.read_file(p))          = tokio::fs::read_to_string(safe_path(p))
interpret(inference.complete(r))    = reqwest::post(ollama_url, r).await
interpret(coord.task_complete(t,r)) = spacetimedb::call("task_complete", [t, r])
interpret(brain.parse_intent(s))    = QLearningBrain::parse_intent(s)
```

This morphism is unique given the adapter implementations (universal property
of the free algebra). Swapping an adapter is choosing a different morphism
that agrees on its sub-signature but differs in interpretation.

### Why This Matters

1. **Testing is morphism substitution.** A mock is a different morphism `interpret_test: T(Sigma) -> TestOutput`. It must satisfy the same invariants.

2. **Architecture violations are signature violations.** A domain term referencing `Sigma_inf` is a type error in the algebra.

3. **Adapter swap is provably safe.** "Ollama and Anthropic are interchangeable" = both are valid interpretations of `Sigma_inf`.

4. **Generative testing.** Given `Sigma_L`, generate random terms, run them through the real interpreter and a mock, check invariant agreement.

---

## Summary Table

| Port | Symbol | Ops | Async | Error Type | Effect Class |
|------|--------|-----|-------|------------|-------------|
| IFileSystemPort | Sigma_fs | 5 | yes | FileSystemError | Disk I/O |
| IInferencePort | Sigma_inf | 4 | yes | InferenceError | Network I/O |
| ICoordinationPort | Sigma_coord | 11 | yes | CoordinationError | Distributed state |
| IEnforcementPort | Sigma_enf | 1 | no | (infallible) | Pure |
| ISecretPort | Sigma_secret | 4 | yes | SecretError | Vault I/O |
| IBrainPort | Sigma_brain | 6 | no | Box\<dyn Error\> | Pure + mutable state |
| IBuildPort | Sigma_build | 4 | no | (in BuildOutput) | Subprocess |
| ISandboxPort | Sigma_sandbox | 4 | yes | SandboxError | Container lifecycle |
| IAgentRuntimePort | Sigma_agent | 2 | yes | SandboxError | Task dispatch |
| IContextCompressorPort | Sigma_ctx | 2 | no | (infallible) | Pure |
| **Total** | **Sigma** | **43** | | | |

---

## Known Gaps

1. **`hex analyze` checks imports, not invocations.** The Sigma-algebra defines operation-level visibility, but enforcement currently only checks file-level imports.

2. **`IBrainPort` mixes pure and stateful operations.** The Q-table accumulator means this port is not purely algebraic. A follow-up could split it into a pure decision port and a stateful learning port.

3. **`IBuildPort` error encoding.** Failures encoded in `BuildOutput.success` rather than `Result`. Breaks the monadic composition pattern.

4. **Cross-port dependencies.** `IAgentRuntimePort.execute_task` implicitly requires capability grants from `ISecretPort`. Not expressed in the type signature. P5 (effect rows) will formalize this.

5. **No single composition root function.** The morphism `interpret: T(Sigma) -> IO` is distributed across hex-nexus and hex-cli, not a single callable function. Correct by design (the signature is implementation-free), but the morphism is conceptual rather than pointable.

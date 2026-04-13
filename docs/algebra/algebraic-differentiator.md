# Why an AI Operating System Needs Algebraic Foundations

**Last Updated:** 2026-04-12
**ADR:** [ADR-2604111229](adrs/ADR-2604111229-algebraic-formalization-of-process-flow.md)
**Ports Sigma-algebra:** [docs/algebra/ports-signature.md](algebra/ports-signature.md)

---

## The Thesis

An AI Operating System (AIOS) manages agent processes the way Unix manages user processes. Unix needed **process isolation** — without it, one runaway process could corrupt every other. An AIOS needs **algebraic structure** — without it, one rogue agent can violate architecture, deadlock the swarm, leak capabilities, or silently corrupt shared state, and no amount of testing can prove otherwise.

hex is built on this thesis. Its hexagonal architecture isn't a code style — it's a **stratified algebra** where each layer has a formal signature, each agent operates within a provably bounded effect set, and coordination protocols are model-checked for liveness. This is what makes hex an operating system rather than an orchestration script.

---

## The Operating System Analogy

Every real operating system is built on formal invariants. The question is whether they're explicit or implicit.

| OS concept | Unix | AIOS (hex) |
|:---|:---|:---|
| **Process isolation** | Virtual memory — hardware MMU enforces address space boundaries | Hexagonal boundaries — Sigma-algebra enforces operation-space boundaries |
| **Capability model** | File descriptors — process can only access fds it was granted | Effect rows — agent can only invoke operations in its capability grant |
| **Scheduling** | Process state machine (ready → running → blocked → zombie) | Workflow Petri net (specs → plan → code → validate → integrate → finalize) |
| **IPC** | Signals, pipes, sockets — formally specified POSIX semantics | SpacetimeDB reducers — pi-calculus model, TLA+ checked for deadlock freedom |
| **Resource reclamation** | Zombie reaping, OOM killer | Heartbeat timeout → dead agent → task reclamation (liveness proof) |

The parallel is not decorative. Unix's formal invariants (virtual memory isolation, POSIX signal semantics, file descriptor capability model) are what made it possible to run untrusted user programs safely. hex's formal invariants (Sigma-algebra boundaries, effect rows, workflow nets, pi-calculus coordination) are what make it possible to run untrusted AI agents safely.

**Without these invariants, you don't have an operating system. You have a shell script that launches processes and hopes for the best.** That's what every other agent framework is.

---

## Why Agents Are Harder Than Processes

Traditional processes are bad enough — but AI agents are worse along every axis that matters for an OS:

**1. Agents are non-deterministic by construction.**
A Unix process given the same input produces the same output (modulo signals and IO). An LLM agent given the same prompt produces different output every time. This means you cannot test agent coordination by replaying traces — the trace will diverge on the next run. You need **invariants that hold across all possible agent outputs**, not tests that hold for one specific output.

Algebraic structure gives you this. The Sigma-algebra says "regardless of what the inference port returns, the domain layer cannot call it." The Petri net says "regardless of how long each phase takes, Tier-3 cannot start before Tier-1 finishes." These are universally quantified properties. Tests are existentially quantified.

**2. Agents have unbounded effect surfaces.**
A Unix process can only do what its syscall interface permits. An AI agent with tool access can read files, write files, spawn processes, make network requests, modify its own prompts, and call other agents. Without a formal effect boundary, the agent's capability set is "everything the host machine can do."

The Sigma-algebra partitions the effect surface into 10 named signatures with 43 total operations. Each layer sees only its permitted sub-signature. An agent running in the domain layer literally cannot express a filesystem write — it's not in the signature. This is stronger than a runtime check; it's a structural impossibility.

**3. Agent coordination has combinatorial state spaces.**
Two Unix processes sharing a pipe have a tractable number of interleavings. Five AI agents sharing a task queue, each with heartbeat timeouts, crash recovery, and CAS-based task claims, have thousands of possible interleavings. Testing a handful is meaningless — the deadlock lives in interleaving #4,721 that your test harness never generated.

The pi-calculus / TLA+ specification checks **all** interleavings. Not a sample. All of them. That's what model checking does. It's exhaustive verification over the finite state space. No agent framework in existence does this — they test the happy path and ship.

**4. Agent failures are silent.**
A segfaulting process produces a core dump. An agent that generates architecturally wrong code produces... code that compiles. The violation is invisible until a human reads it, or until a downstream agent builds on the wrong foundation and the error compounds.

The enforcement port (`Sigma_enf`) is a pure guard function that sits in front of every effectful operation. In `Mandatory` mode, a boundary violation blocks the operation before it executes. But the deeper protection is the algebra itself: `hex analyze` can check not just "did this file import the wrong module" but "does this term reference an operation outside its layer's sub-signature." That catches violations that pass both the compiler and the import linter.

---

## What the Algebra Stack Proves

hex models its process flow as four independent algebras — one per architectural concern. Each uses a different 30-50 year old formalism with mature tooling. No grand unified theory. No category-theory prerequisite.

```
┌─────────────────────────────┬────────────────────────────────────────────┐
│  Concern                    │  Formalism            │  What it proves    │
├─────────────────────────────┼───────────────────────┼────────────────────┤
│  Effect boundaries          │  Free Sigma-algebra   │  Agents only use   │
│  (who can do what)          │  + effect row types   │  permitted ops     │
├─────────────────────────────┼───────────────────────┼────────────────────┤
│  Dispatch pipeline          │  Kleisli composition  │  Pipeline shape,   │
│  (how work gets routed)     │  over Result          │  short-circuit     │
├─────────────────────────────┼───────────────────────┼────────────────────┤
│  Swarm coordination         │  pi-calculus / TLA+   │  Deadlock freedom, │
│  (agents talking to agents) │                       │  no task loss      │
├─────────────────────────────┼───────────────────────┼────────────────────┤
│  Feature lifecycle          │  1-safe workflow      │  Reachability,     │
│  (how features get built)   │  Petri net            │  phase ordering    │
└─────────────────────────────┴───────────────────────┴────────────────────┘
```

### The Sigma-algebra: Effect Boundaries

10 port traits, 43 operations. Each port is a sort in the algebraic signature. Each layer of the hexagon sees a different sub-signature:

- **Domain:** `Sigma_domain = empty` — domain code has zero effect operations
- **Adapters:** `Sigma_adapter(P) = Sigma_P` — each adapter sees only its own port
- **Use cases:** `Sigma_usecases = Sigma` — full access via injected trait objects
- **Composition root:** `Sigma` — the unique morphism `T(Sigma) -> IO`

The composition root is the interpreter. Swapping an adapter (Ollama for Anthropic) is choosing a different morphism that agrees on `Sigma_inf`. This is the universal property of the free algebra — and it's exactly what "dependency injection" means, stated precisely.

### The pi-calculus: Swarm Safety

Agents are processes. SpacetimeDB reducers are channels. Heartbeat timeouts are timed transitions. The TLA+ spec checks:

- **Deadlock freedom:** No reachable state where all agents are blocked
- **No task loss:** Crashed agent's task is always reclaimed
- **CAS correctness:** Exactly one agent wins a race to claim a task

### The Petri Net: Lifecycle Soundness

The 7-phase pipeline with fork/join parallelism in the Code phase. The net proves:

- Every started workflow reaches completion (soundness)
- No phase is unreachable (no dead transitions)
- Tier barriers hold under parallel dispatch (ordering guarantee)

---

## Why No One Else Can Bolt This On

The algebraic formalization works for hex because hex was **designed as layered architecture from day one**. The Sigma-algebra doesn't impose structure — it reveals structure that's already there. The hexagonal boundary rules *are* sub-signature inclusions. The composition root *is* the interpretation morphism. The enforcement port *is* a guard in the Kleisli pipeline.

Other frameworks cannot retrofit this because they don't have the layers:

| Framework | Why it can't be formalized |
|:---|:---|
| **LangChain** | Tools are a runtime list, not a typed signature. No layer boundaries — chains call tools, tools call chains, callbacks modify state from anywhere. |
| **CrewAI** | Agents have role strings, not effect types. Delegation is a method call, not a channel. No isolation between agents — they share memory by default. |
| **AutoGen** | Group chat is a conversation loop, not a coordination protocol. No task state machine. Agent-to-agent communication has no formal semantics. |
| **Claude Agent SDK** | Tool-first, but tools are defined at runtime with no layer stratification. No lifecycle model — agents run until they stop. |
| **SPECkit** | Spec documents are prose, not algebraic signatures. No runtime enforcement — the spec says what should happen, nothing prevents violations. |

To formalize these systems, you'd first have to **refactor them into layers**, then **define signatures for each layer**, then **prove the layer boundaries hold**. That's a rewrite, not a patch.

hex is already there because the hexagonal architecture *is the algebra*.

---

## The AIOS Argument, Summarized

An operating system earns that title by providing **formal guarantees about process behavior** — not by launching processes and observing what happens. Unix provides memory isolation, capability-based file access, and POSIX-defined IPC semantics. Programs that violate these invariants are stopped by the kernel, not by tests.

An AI Operating System must provide the same class of guarantees for AI agents:

1. **Effect isolation** — agents can only invoke operations they're permitted (Sigma-algebra)
2. **Coordination safety** — multi-agent protocols are deadlock-free and liveness-guaranteed (pi-calculus / TLA+)
3. **Lifecycle soundness** — the development pipeline always reaches completion and respects ordering (Petri net)
4. **Capability confinement** — granted resources cannot escape their scope (effect rows / linear logic)

These are not nice-to-haves. They are the difference between an operating system and a shell script. Every agent framework today is a shell script. hex is the first one that isn't.

---

## How hex Manifests the Proofs

The algebra isn't just documentation. Each layer of the stack is realized as running code — mechanisms in hex that enforce the invariants at build time, at startup, or at operation time. The proofs are alive in the system.

### Sigma-algebra: Trait Boundaries Are the Proof

The Sigma-algebra (effect boundaries) is manifested through **Rust's trait system and hex's composition root pattern**.

**Mechanism:** Each port is a Rust trait (`IFileSystemPort`, `IInferencePort`, etc.). Use cases receive ports as injected `dyn Trait` objects. The Rust compiler itself enforces that a use case can only call methods on the traits it was given — you literally cannot call `fs.write_file()` if your function signature only accepts `&dyn IInferencePort`. The sub-signature inclusion `Sigma_adapter(P) = Sigma_P` is a **type-system guarantee**, not a runtime check.

```rust
// This use case can only invoke Sigma_inf + Sigma_coord — by construction
async fn dispatch_inference(
    inference: &dyn IInferencePort,    // Sigma_inf visible
    coord: &dyn ICoordinationPort,     // Sigma_coord visible
    // IFileSystemPort NOT injected    // Sigma_fs structurally invisible
) -> Result<(), Box<dyn Error>> { ... }
```

**Where it runs:**
- `rustc` enforces at compile time — a use case that tries to call an un-injected port is a compile error
- `hex analyze` enforces at architecture-check time — a file that *imports* a port it shouldn't is flagged
- The composition root in `hex-nexus` / `hex-cli` is the only code that sees all 10 ports simultaneously — it's the interpretation morphism

**What this catches that nothing else does:** An agent generating code inside a use case file cannot accidentally introduce a filesystem dependency if the function signature doesn't accept `IFileSystemPort`. The constraint isn't a linting rule that can be disabled — it's a type error. The algebra is the type system.

### Petri Net: The Supervisor Is the Net

The 7-phase lifecycle Petri net is manifested as **the supervisor's phase gate logic** in `hex-cli/src/pipeline/supervisor.rs`.

**Mechanism:** Each phase transition is gated by a completion check on the previous phase. The Code phase has fork/join semantics — tier-0 (domain) tasks must complete before tier-1 (secondary adapter) tasks fire, which must complete before tier-2, and so on. The supervisor maintains a token count per tier and only advances when the count reaches zero.

```
Phase flow (simplified):

    [SPECS] --ok--> [PLAN] --ok--> [WORKTREES] --ok--> [CODE] --ok--> [VALIDATE] --ok--> [INTEGRATE] --ok--> [FINALIZE]
                                                          |
                                          ┌───────────────┼───────────────┐
                                          v               v               v
                                     [Tier 0: domain] [Tier 1: adapters] [Tier 2: usecases]
                                          |               |               |
                                          └──join──>──────┘──join──>──────┘
```

**Where it runs:**
- `hex plan execute` — the supervisor runs each phase sequentially, with parallel dispatch inside Code
- BLOCKING gates — `VALIDATE` phase must return `PASS` before `INTEGRATE` fires; a `FAIL` halts the pipeline
- `hex plan status` — reports which phase/tier is active, how many tasks remain per tier

**What this catches:** A tier-3 use case agent cannot start coding before tier-0 domain types are committed. This prevents the class of bug where an adapter is coded against an interface that doesn't exist yet. The ordering isn't advisory — the supervisor won't dispatch the agent.

### Pi-calculus: HexFlo Is the Protocol

The pi-calculus (swarm coordination) is manifested as **HexFlo's agent lifecycle and task claim protocol** in `hex-nexus/src/coordination/`.

**Mechanism:** Each agent is a concurrent process with a heartbeat channel. SpacetimeDB reducers are the communication channels — `task_create`, `task_assign`, `task_complete` are atomic reducer calls (serialized by SpacetimeDB's single-writer model). Task claiming uses compare-and-swap: the reducer checks `agent_id IS NULL` before assigning, so two agents racing to claim the same task see a serialized outcome — exactly one wins.

```
Agent lifecycle (state machine):

    [registered] --heartbeat--> [active] --claim--> [working] --complete--> [idle]
         |                         |                    |
         |                    (45s silence)         (crash/timeout)
         |                         v                    v
         |                     [stale]              [dead]
         |                         |                    |
         |                    (120s silence)        reclaim tasks
         |                         v                    |
         └─────────────────────[dead]<──────────────────┘
```

**Where it runs:**
- `hex hook route` sends heartbeat on every `UserPromptSubmit`
- `hex-nexus/src/coordination/cleanup.rs` runs the stale/dead detection loop
- SpacetimeDB's `task_assign` reducer implements CAS — `UPDATE swarm_task SET agent_id = ? WHERE id = ? AND agent_id IS NULL`
- `hex-nexus/src/coordination/mod.rs` orchestrates task reclamation from dead agents

**What this catches:** Task loss. If agent A crashes while holding task T, the heartbeat timeout fires, A is marked dead, T is returned to the unassigned pool, and another agent can claim it. The CAS prevents double-assignment. These properties hold for any number of agents and any crash timing — they're protocol invariants, not test-case-specific outcomes.

### Kleisli Pipeline: The Hook Router Is the Composition

The Kleisli composition (tier routing) is manifested as **the classify-dispatch pipeline** in `hex-cli/src/commands/hook.rs`.

**Mechanism:** Every user prompt passes through `classify_work_intent()` — a pure function that scores the prompt against T1/T2/T3 heuristics and returns a tier. The downstream dispatch is a chain of `Result`-returning functions composed with `?` (Rust's monadic bind). If classification returns T1, dispatch short-circuits and the prompt is handled inline. If T3, it flows through to workplan draft creation.

```rust
// The Kleisli chain — each step returns Result, ? is monadic bind
let tier = classify_work_intent(&prompt)?;     // Pure: String -> Result<Tier>
let action = dispatch_tier(tier, &ctx)?;        // Pure: Tier -> Result<Action>
let healed = heal_if_needed(action, &state)?;   // Pure: Action -> Result<Action>
persist_action(healed, &store)?;                // Effectful: Action -> Result<()>
```

**Where it runs:**
- `hex hook route` on every `UserPromptSubmit` event
- The classifier is tested as a pure function with deterministic inputs
- Short-circuit: T1 prompts (questions, typo fixes) never reach the workplan machinery

**What this catches:** Tier misclassification cascading into unwanted side effects. A question prompt ("how does X work?") classified as T3 would spawn a workplan draft — an unwanted, visible side effect. The Kleisli structure means the short-circuit is structural, not a conditional branch buried in a 500-line function.

### Effect Rows: Capability Grants Are the Confinement

The effect row types (P5, in design) are partially manifested today through **the secret-grant system and enforcement port**.

**Mechanism:** Before an agent can access a secret, it must have a `SecretGrant` issued to it with a TTL. The grant is a **linear resource** — `claim_secrets()` can only be called once per agent (subsequent calls return `AlreadyClaimed`). The enforcement port (`IEnforcementPort.check()`) runs before every effectful operation and can `Block` operations that exceed the agent's granted capabilities.

**Where it runs:**
- `hex secrets grant` / `hex secrets revoke` — explicit capability management
- `hex-nexus` middleware — enforcement check before file writes, agent spawns, bash execution
- Secret TTL expiry — grants auto-revoke after their time window

**What this catches today:** An agent spawned for a specific task can be granted `FileSystem("/src/adapters/http")` — it can read/write within that path but nowhere else. Combined with the `PathTraversal` check in `IFileSystemPort`, this is a two-layer defense: the grant controls what the agent is *allowed* to do, the port invariant controls what the *operation* can physically reach.

**What P5 will add:** Compile-time verification that an agent's task requirements are a subset of its capability grants — before the agent is dispatched, not after it tries an unauthorized operation.

---

## The Full Picture

```
                    ┌──────────────────────────────────────────┐
                    │           hex AIOS                       │
                    │                                          │
  User prompt ─────>│  Kleisli pipeline (classify -> dispatch) │
                    │       |                                  │
                    │       v                                  │
                    │  Enforcement guard (Sigma_enf.check)     │
                    │       |                                  │
                    │       v                                  │
                    │  Petri net supervisor (phase gates)       │
                    │       |                                  │
                    │       v                                  │
                    │  HexFlo pi-calculus (agent dispatch)      │
                    │       |                                  │
                    │       v                                  │
                    │  Sigma-algebra boundary (trait injection) │
                    │       |                                  │
                    │       v                                  │
                    │  Composition root (morphism into IO)      │
                    │       |                                  │
                    │       v                                  │
                    │  [Adapters: Ollama, SpacetimeDB, FS...]  │
                    └──────────────────────────────────────────┘
```

Every layer in this stack provides a **different class of guarantee**:

1. The **Kleisli pipeline** guarantees the prompt reaches the right handler (no misrouting)
2. The **enforcement guard** guarantees the operation is permitted (no unauthorized effects)
3. The **Petri net** guarantees phases execute in order (no premature dispatch)
4. The **pi-calculus protocol** guarantees agents coordinate safely (no deadlock, no task loss)
5. The **Sigma-algebra** guarantees code stays within its layer (no boundary violations)
6. The **composition root** guarantees the abstract program maps to concrete effects (no dangling abstractions)

Strip any one layer and you lose a class of guarantee that the remaining layers cannot compensate for. This is why hex is an operating system — it's a stack of formal invariants, each catching failures invisible to the others.

---

> hex is the only AI agent system where "the swarm can't deadlock" is a **theorem**, not a **hope**.

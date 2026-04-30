# ADR-2604301215: Vera-Class Reducer DSL for SpacetimeDB Modules

**Status:** Proposed (long-horizon — design sketch only, no implementation work scheduled)
**Date:** 2026-04-30
**Drivers:** Same design discussion as ADR-2604301200. The 7 SpacetimeDB modules in `spacetime-modules/` are the coordination kernel everything else trusts: agent-registry, hexflo-coordination, secret-grant, inference-gateway, neural-lab, rl-engine, chat-relay. They are written in Rust, which gives memory safety and a strong type system but no contract verification. Reducer pre/post conditions are encoded as runtime `Result::Err` returns and prose comments — exactly the shape that maps cleanly onto a Vera-class contract language. They are also the place where verification pays the most: small files, contract-heavy logic, low edit frequency, high blast radius if wrong.
**Supersedes:** None (companion to ADR-2604301200)

<!-- ID format: YYMMDDHHMM — 2604301215 = 2026-04-30 12:15 local -->

## Context

`spacetime-modules/secret-grant/src/lib.rs` is representative. The `claim_grant` reducer reads:

```rust
#[reducer]
pub fn claim_grant(
    ctx: &ReducerContext,
    agent_id: String,
    secret_key: String,
    claim_hub_id: String,
    claimed_at: String,
) -> Result<(), String> {
    let id = format!("{}:{}", agent_id, secret_key);
    match ctx.db.secret_grant().id().find(&id) {
        Some(existing) => {
            if existing.claimed {
                return Err(format!("Grant '{}' for agent '{}' already claimed", ...));
            }
            ctx.db.secret_grant().id().update(SecretGrant {
                claimed: true, claimed_at, claim_hub_id, ..existing
            });
            Ok(())
        }
        None => Err(format!("No grant for key '{}' found ...")),
    }
}
```

Read literally, this reducer has three contracts the Rust compiler does not check:

1. **Precondition: the grant exists.** Currently a runtime `Err` if violated.
2. **Precondition: the grant is unclaimed.** Currently a runtime `Err` if violated.
3. **Postcondition: after success, the grant is claimed and `claimed_at` is set.** Not checked at all — a future refactor that forgets to set `claimed: true` would compile.

There is also a state invariant the *whole table* should uphold — "a claimed grant has non-empty `claimed_at`" — which today lives in the heads of the people who wrote ADR-026. None of these are mechanically enforced.

WASM-no-ambient-capability already gives SpacetimeDB modules half of what Vera promises: no FS, no network, no spawn. The structural side of the effect-row check is free. What's missing is the contract side and the totality side.

### Why a DSL rather than Rust attribute macros

Three reasons:

1. **SMT discharge wants a small, fixed surface.** `creusot` and `prusti` can verify Rust, but their fragment is large, fragile across rustc versions, and far from production-ready as of 2026-04. A purpose-built reducer DSL with a smaller AST gives a tractable encoding to Z3.
2. **The reducer surface is already a domain-specific shape.** Every reducer is `(ctx, args...) -> Result<(), String>`. State access is `ctx.db.<table>()...`. Effects are exactly the set `{ DbRead, DbWrite, Log }`. Encoding this in a DSL is shorter than encoding it in attribute macros over Rust.
3. **Generation-time verification is the goal.** When an LLM writes a reducer, the obligation that it satisfies the contract is what we want, not "the Rust compiler accepted the macro expansion." A DSL keeps the model writing inside the verified fragment by construction.

### Alternatives considered

1. **Use creusot/prusti.** Rejected for now (research-stage, large surface) — revisit after 2027 if the ecosystem matures.
2. **Hand-write contracts as `#[contract(...)]` attribute macros that lower to Rust assertions.** Gives runtime checks but not generation-time verification. Acceptable as a stepping stone but doesn't deliver the value.
3. **Vera-class DSL that compiles to the existing `#[reducer]` Rust.** Chosen. The compile target is exactly the macros SpacetimeDB already accepts, so the runtime is unchanged; verification is purely a build-time gate.

## Decision

A Vera-class reducer DSL — working name `hex-reducer` — SHALL be designed (not yet built) with the surface and obligations below. Compile target is `#[reducer]` Rust as emitted by the existing `spacetimedb` crate. Verification target is Z3.

This ADR records the design; implementation requires a separate workplan and is not scheduled. The intent is to (a) anchor the design while ADR-2604301200 makes the team fluent in `requires`/`ensures` at the workplan layer, (b) give a concrete artifact future ADRs can reference, and (c) check the design against real reducers before committing to build.

### Surface syntax sketch (illustrative — not final)

```vera
module secret_grant {

  // ─── Tables (lower to #[table] structs) ────────────────────────────────

  table secret_grant @private {
    id           : String  @unique
    agent_id     : String
    secret_key   : String
    purpose      : String
    hub_id       : String
    granted_at   : Iso8601
    expires_at   : Iso8601
    claimed      : Bool
    claimed_at   : Iso8601 | Empty
    claim_hub_id : String
  }

  // Table-level invariant — Z3-checked across every reducer that mutates the table.
  invariant on secret_grant {
    @row.id == @row.agent_id ++ ":" ++ @row.secret_key
    && (@row.claimed ⇒ @row.claimed_at != Empty)
    && (@row.claimed ⇒ @row.claim_hub_id != "")
  }

  // ─── Reducers (lower to #[reducer] fns) ────────────────────────────────

  reducer claim_grant(
    agent_id      : String,
    secret_key    : String,
    claim_hub_id  : String,
    claimed_at    : Iso8601,
  ) {
    let id = agent_id ++ ":" ++ secret_key in

    requires {
      exists g in db.secret_grant where g.id == id           // grant exists
      ∧ ¬(the g in db.secret_grant where g.id == id).claimed // and is unclaimed
      ∧ claim_hub_id != ""                                   // structural
    }

    ensures {
      let g = the g in db.secret_grant where g.id == id in
      g.claimed == true
      ∧ g.claimed_at == claimed_at
      ∧ g.claim_hub_id == claim_hub_id
      // frame condition: nothing else changes
      ∧ ∀ other in db.secret_grant where other.id != id . other == @pre(other)
    }

    effects { DbRead(secret_grant), DbWrite(secret_grant) }

    body = update db.secret_grant
           where id == id
           set { claimed = true, claimed_at, claim_hub_id }
  }

  reducer prune_expired(now: Iso8601) {
    requires { true }

    ensures {
      // every remaining grant is still in-date
      ∀ g in db.secret_grant . g.expires_at > now
      // nothing in-date was removed
      ∧ ∀ g in @pre(db.secret_grant) where g.expires_at > now .
            exists g' in db.secret_grant where g' == g
    }

    effects { DbRead(secret_grant), DbWrite(secret_grant), Log }

    body = delete from db.secret_grant where expires_at <= now
  }
}
```

### Obligations the toolchain discharges

For each reducer:

```
table_invariant(@pre)
  ∧ requires(@pre, args)
  ⊢ table_invariant(@post)
    ∧ ensures(@pre, @post, args)
    ∧ effects(body) ⊆ effects_declared
    ∧ terminates(body, decreases)
```

`@pre` and `@post` name the table state before and after the reducer. The frame-condition idiom (`∀ other ... other == @pre(other)`) is built-in syntax to keep simple frames cheap to write.

### Vera-shape rules carried over

- **No mutation outside `update`/`insert`/`delete` on declared tables.** All other identifiers are immutable.
- **Exhaustive match** on enum types.
- **Recursion needs `decreases`.** Reducers themselves don't typically recurse; helpers in the module do, and Vera's standard well-founded measure applies.
- **Effect rows are mandatory.** Allowed effects: `DbRead(t)`, `DbWrite(t)`, `Log`. No FS, no Net — and crucially, no way to *spell* either, because the WASM target can't host them. The structural check is for free.
- **No ambient capabilities.** Tables are addressed by name, not by handle, because SpacetimeDB scopes them to the module. There is no equivalent to a "raw pointer" escape hatch.

### What this is NOT

- **Not a replacement for Rust at the hex-nexus / hex-cli layer.** Those are I/O-heavy, ecosystem-dependent, and exactly the wrong fit for Vera-class constraints. Rust stays.
- **Not a replacement for the existing `#[reducer]` macros at runtime.** `hex-reducer` compiles *to* them. SpacetimeDB sees normal Rust; the verification is upstream.
- **Not a contract language for arbitrary Rust.** Scoped to the reducer surface. Helpers used by reducers must be expressible in the DSL or be from a verified standard library.

### Compile pipeline

```
.vera reducer source
   │  parse
   ▼
typed AST + contract obligations
   │  SMT discharge (Z3)
   ▼  (fail → diagnostic shaped as instructions to a model, per Vera convention)
codegen
   │
   ▼
Rust source compatible with `spacetimedb` crate's #[reducer]/#[table]
   │  cargo build --target wasm32-unknown-unknown
   ▼
.wasm published via `spacetime publish`
```

Diagnostic format follows Vera's "instructions to a model" convention: when an obligation fails, the toolchain emits a structured message describing which precondition/postcondition failed and which SMT counterexample produced the failure, formatted as a prompt fragment that a model can use directly to revise the body. This is the generation-time feedback loop Rust's compiler was not designed for.

### Open questions (deliberately unresolved)

1. **Aggregate queries.** `the g in db.secret_grant where ...` requires uniqueness reasoning. Either restrict to `@unique`-keyed lookups, or add cardinality types (`Set<T>` vs `Option<T>` vs `T`) at the AST level.
2. **Cross-module contracts.** When `inference-gateway` calls into `secret-grant`, does the caller see the callee's `requires`? Probably yes for reducers exported to other modules, but mechanism is undecided.
3. **Standard library scope.** Vera's 137 functions are deliberately spartan. The reducer DSL needs at least string concatenation, ISO8601 comparison, Option/Result, and basic collection predicates. Defining the exact set is a separate ADR.
4. **Contract retrofit path.** Existing 7 modules contain ~40 reducers. Migration is a workplan, not a flag flip; some reducers (especially in `neural-lab` and `rl-engine`) may not fit the verified fragment without redesign.

## Consequences

**Positive:**
- The kernel of trust gets generation-time verification. ADR-026's threat model (secret grants, audit log) becomes provable at build time, not just defended in prose.
- LLM authoring of reducers becomes high-yield. The verifier is the loop the model iterates against; "compiles" is replaced with "compiles and verifies," which is the actual quality signal.
- Effect-row checks for free. Because WASM can't host FS or Net, declaring effects is just ergonomic — the structural guarantee is already there.
- Frees Rust to be Rust at the I/O boundary. The split (Vera inside the kernel, Rust at the daemon, the same host runtime) matches the architectural shape we already have. The language pluralism becomes principled rather than accidental.

**Negative:**
- New language. Tooling, editor support, model fluency, documentation — all need to exist. Cost is real; mitigated by deliberately not building until ADR-2604301200 is shipped and the team has lived with `requires`/`ensures` at the workplan layer.
- Vera-class constraints (no mutation, total functions) bite some existing reducers. `rl-engine` in particular has loop-shaped logic that would need rewriting in a fold-shaped style.
- SMT solvers are imperfect; some honest contracts will fail to discharge automatically and need either a manual proof hint or contract weakening. Mitigated by keeping the DSL fragment small enough that Z3's success rate is empirically high.
- Ecosystem isolation. Vera-class languages have no Cargo, no crates.io. We accept this for the kernel because the kernel deliberately has no third-party dependencies anyway.

**Mitigations:**
- Implementation is gated on ADR-2604301200 shipping and on a 6-month track record of `requires`/`ensures` working at the workplan layer. If predicates at the workplan layer are ignored or routinely escape-hatched to `CommandSucceeds`, that's the signal not to build the DSL.
- Parallel-track migration: a new module gets written in `hex-reducer`, the existing 7 stay in Rust until each is justified to port.
- Diagnostic-as-prompt format is the killer feature for LLM authorship; invest there before generality.

## Implementation

This ADR records the design; **no implementation phases are scheduled**. The work below is a placeholder for a future workplan once the prerequisites are met.

| Phase | Description                                                                                            | Status         |
|-------|--------------------------------------------------------------------------------------------------------|----------------|
| D0    | Prerequisite: ADR-2604301200 in production for ≥ 6 months with `verify_fail` rate measured             | Not started    |
| D1    | Frozen DSL grammar + AST + Z3 encoding for the secret-grant reducers as the proof case                 | Design only    |
| D2    | Pilot port: rewrite `secret-grant` in `hex-reducer`, keep `agent-registry` in Rust as control          | Not scheduled  |
| D3    | Diagnostic-as-prompt format + integration with hex-coder agent                                         | Not scheduled  |
| D4    | Migration decision per remaining module based on D2 evidence                                           | Not scheduled  |

## References

- ADR-2604301200 — contract-typed WorkplanTask + verify-gate (the near-term prerequisite)
- ADR-026 — secret grant threat model (the contracts this DSL would mechanise)
- ADR-2604050900 — 7 SpacetimeDB modules layout
- ADR-2604120202 — tiered inference routing (the loop a Vera diagnostic plugs into)
- CLAUDE.md "Key Lessons" — "It compiles ≠ It works" → the gap a Vera-class verifier closes

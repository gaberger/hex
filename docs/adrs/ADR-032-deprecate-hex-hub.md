# ADR-032: Deprecate hex-hub — Consolidate into hex-nexus and hex-agent

| Field | Value |
|-------|-------|
| **Status** | Accepted |
| **Date** | 2026-03-18 |
| **Supersedes** | ADR-016 (Hub Binary Version Verification) |
| **Affects** | ADR-011, ADR-015, ADR-024, ADR-025, ADR-026, ADR-027 |
| **Decision** | Deprecate `hex-hub/` crate; absorb its code into `hex-nexus` (binary target) and `hex-agent` |

---

## Context

The hex project currently ships **three Rust crates**:

| Crate | Type | LOC | Purpose |
|-------|------|-----|---------|
| `hex-nexus` | Library | 25,729 | Core orchestration: routes, HexFlo, RL, persistence, fleet, secrets, chat |
| `hex-hub` | Binary | 1,517 | Thin wrapper: CLI args, `build_app()` call, `IStatePort` trait + 2 adapters |
| `hex-agent` | Binary | ~8,700 | Autonomous AI agent: conversation loop, tool execution, multi-provider inference |

**hex-hub is architecturally hollow.** Its `main.rs` is 57 lines — it parses CLI args, calls `hex_nexus::build_app()`, and binds to a TCP port. The only substantial code is `IStatePort` (329 LOC) and two adapter implementations (SQLite: 663 LOC, SpacetimeDB: 463 LOC) that logically belong in hex-nexus.

This creates several problems:

1. **Confusing naming**: The binary was already renamed from `hex-hub` to `hex-nexus` (commit 8f4a317), but the crate directory is still `hex-hub/`. Users see a `hex-nexus` binary produced by a `hex-hub` crate that depends on a `hex-nexus` library.

2. **Artificial code split**: The `IStatePort` trait — the primary abstraction for ALL persistence — lives in the binary crate, not the library. This means hex-nexus (the library) cannot use its own state port without a circular dependency. Tests that need state access must live in hex-hub.

3. **Build complexity**: Two Cargo crates, two `build.rs` files, two release profiles, two sets of dependencies — for what is effectively one deployable unit.

4. **ADR drift**: ADR-024 promoted hex-hub to "autonomous nexus," but hex-agent has since absorbed the autonomous agent runtime. hex-hub's remaining role is just "start the HTTP server."

---

## Decision

**Deprecate `hex-hub/` as a separate crate.** Migrate all code into `hex-nexus` and `hex-agent`:

### What Moves Where

| Current Location (hex-hub) | Destination | Rationale |
|---------------------------|-------------|-----------|
| `src/main.rs` (57 LOC) | `hex-nexus/src/bin/hex-nexus.rs` | Binary entry point becomes a bin target in the library crate |
| `src/ports/state.rs` (329 LOC) | `hex-nexus/src/ports/state.rs` | State port belongs with the library that defines all routes using it |
| `src/adapters/sqlite_state.rs` (663 LOC) | `hex-nexus/src/adapters/sqlite_state.rs` | SQLite adapter stays with the server |
| `src/adapters/spacetime_state.rs` (463 LOC) | `hex-nexus/src/adapters/spacetime_state.rs` | SpacetimeDB adapter stays with the server |
| `build.rs` (20 LOC) | `hex-nexus/build.rs` | Build hash generation |
| `Cargo.toml` dependencies | Merged into `hex-nexus/Cargo.toml` | `tracing-subscriber` is the only new dep |

### What hex-agent Gains

hex-agent is already independent and needs **no changes** from this deprecation. It connects to the hex-nexus server via WebSocket — the server binary just moves from `hex-hub/` to `hex-nexus/src/bin/`.

### What Gets Deleted

The entire `hex-hub/` directory (1,517 LOC) after migration is verified.

---

## Analysis

### Capability Inventory

#### hex-hub Capabilities (ALL transfer to hex-nexus)

| Capability | LOC | Migration Complexity |
|------------|-----|---------------------|
| CLI arg parsing (port, bind, token, daemon) | 57 | Trivial — copy to bin target |
| `IStatePort` trait definition | 329 | Low — move file, update imports |
| `SqliteStateAdapter` | 663 | Low — move file, update imports |
| `SpacetimeStateAdapter` | 463 | Low — move file, update imports (feature-gated) |
| Build hash embedding | 20 | Trivial — merge build.rs |
| **Total** | **1,537** | **Low** |

#### hex-nexus Capabilities (unchanged, now includes binary)

| Capability | Module | LOC |
|------------|--------|-----|
| HTTP/WS routing (54+ endpoints) | `routes/` | ~2,500 |
| HexFlo swarm coordination | `coordination/` | ~800 |
| Agent management | `orchestration/agent_manager.rs` | ~400 |
| Workplan execution | `orchestration/workplan_executor.rs` | ~600 |
| RL engine (Q-learning) | `rl/` | ~500 |
| Swarm persistence (SQLite) | `persistence.rs` | ~700 |
| Fleet management (SSH) | `remote/` | ~500 |
| Secret broker | `routes/secrets.rs` | ~300 |
| Chat relay (WebSocket) | `routes/chat.rs` | ~200 |
| Embedded dashboard assets | `embed.rs` + `assets/` | ~100 |
| Multi-instance coordination | `routes/coordination.rs` | ~400 |
| SpacetimeDB bindings | `spacetime_bindings/` | ~3,000 |
| Background cleanup tasks | `cleanup.rs` + `lib.rs` | ~200 |
| Auth middleware | `middleware/auth.rs` | ~100 |
| **Subtotal (library)** | | **~10,300** |
| **+ IStatePort & adapters (from hex-hub)** | | **+1,455** |
| **+ Binary entry point (from hex-hub)** | | **+77** |
| **New total** | | **~27,250** |

#### hex-agent Capabilities (independent, no changes needed)

| Capability | Module | Status |
|------------|--------|--------|
| Anthropic Messages API | `adapters/secondary/anthropic.rs` | Production |
| OpenAI-compat providers | `adapters/secondary/openai_compat.rs` | Production |
| Tool execution (bash, git, fs) | `adapters/secondary/tools.rs` | Production |
| RL-driven model selection | `adapters/secondary/rl_client.rs` | Production |
| Prompt caching | `domain/api_optimization.rs` | Production |
| Extended thinking | `domain/api_optimization.rs` | Production |
| Auto-compaction | `usecases/context_packer.rs` | Production |
| Hub WebSocket relay | `adapters/secondary/hub_client.rs` | Production |
| Secret claim (hub or env) | `adapters/secondary/hub_claim_secrets.rs` | Production |
| Haiku preflight | `adapters/secondary/haiku_preflight.rs` | Production |
| Token metrics | `adapters/secondary/token_metrics.rs` | Production |
| Config migration | `adapters/primary/migrate.rs` | Production |

---

### Benefits

1. **Eliminates naming confusion**: One crate (`hex-nexus`) produces one binary (`hex-nexus`). No more "hex-hub crate that builds hex-nexus binary."

2. **Reunites state port with its consumers**: `IStatePort` moves into the library where all 54 route handlers that USE it already live. This enables proper unit testing of routes with mock state.

3. **Simpler builds**: One `cargo build` instead of two. One release profile. One `Cargo.toml` to maintain.

4. **Cleaner dependency graph**:
   ```
   BEFORE:  hex-hub → hex-nexus (library)
                       hex-agent → hex-nexus (optional, feature-gated)

   AFTER:   hex-nexus (library + binary)
            hex-agent → hex-nexus (optional, feature-gated)
   ```

5. **Reduces CI time**: One fewer crate to compile, link, and test.

6. **Aligns with ADR-024 intent**: ADR-024 declared hex-hub the "nexus." Now the nexus IS hex-nexus, literally.

### Risks

| Risk | Severity | Mitigation |
|------|----------|------------|
| Breaking existing `~/.hex/bin/hex-hub` symlinks | Medium | Install script creates `hex-hub` → `hex-nexus` symlink for 2 releases |
| Cargo workspace changes may break CI | Low | Test in worktree before merging |
| `IStatePort` move changes public API surface of hex-nexus | Low | It was never public — hex-nexus was lib-only, consumed only by hex-hub |
| SpacetimeDB feature flag complexity | Low | Feature stays identical, just moves crates |
| hex-agent's conditional `hex-nexus` import | Low | Import path unchanged — hex-nexus is still a library |

### Non-Risks

- **hex-agent is unaffected**: It connects via WebSocket to whatever address is running. The binary name is already `hex-nexus`.
- **MCP tools are unaffected**: They hit HTTP endpoints, not Rust crate boundaries.
- **Dashboard assets are unaffected**: Already in `hex-nexus/assets/`, already embedded by hex-nexus.
- **TypeScript CLI is unaffected**: It spawns the `hex-nexus` binary by name (already renamed).

---

## Migration Plan

### Phase 1: Move IStatePort into hex-nexus (non-breaking)

```
1. Copy hex-hub/src/ports/state.rs      → hex-nexus/src/ports/state.rs
2. Copy hex-hub/src/adapters/sqlite_*   → hex-nexus/src/adapters/sqlite_state.rs
3. Copy hex-hub/src/adapters/spacetime_* → hex-nexus/src/adapters/spacetime_state.rs
4. Update hex-nexus/src/ports/mod.rs and adapters/mod.rs
5. Update hex-nexus imports to use local state port
6. hex-hub/src/main.rs imports from hex-nexus instead of local ports
7. Run full test suite
```

**Outcome**: hex-hub becomes a ~30 LOC binary (just main + CLI args). All logic is in hex-nexus.

### Phase 2: Add binary target to hex-nexus

```
1. Create hex-nexus/src/bin/hex-nexus.rs (from hex-hub/src/main.rs)
2. Add [[bin]] section to hex-nexus/Cargo.toml
3. Merge hex-hub/build.rs into hex-nexus/build.rs
4. Add tracing-subscriber to hex-nexus/Cargo.toml
5. Verify: cargo build produces hex-nexus binary
6. Run full test suite + integration tests
```

**Outcome**: hex-nexus crate produces both a library AND a binary. hex-hub is now redundant.

### Phase 3: Deprecate and remove hex-hub

```
1. Mark hex-hub/Cargo.toml as deprecated (add [package] description note)
2. Update workspace Cargo.toml to remove hex-hub member
3. Update all documentation (CLAUDE.md, README, ADRs) to reference hex-nexus only
4. Update install scripts to remove hex-hub references
5. Add hex-hub → hex-nexus symlink in install script (backward compat)
6. Delete hex-hub/ directory
7. Update ADR-016 status to "Superseded by ADR-031"
```

**Outcome**: Single crate, single binary, clean architecture.

### Phase 4: Update affected ADRs

| ADR | Update Needed |
|-----|--------------|
| ADR-011 | Change "hex-hub HTTP endpoints" → "hex-nexus HTTP endpoints" |
| ADR-015 | Change "Hub Swarm State" → "Nexus Swarm State" |
| ADR-016 | Mark superseded — build hash moves to hex-nexus/build.rs |
| ADR-024 | Update architecture diagram to show consolidated hex-nexus |
| ADR-025 | Change "hex-hub IStatePort" → "hex-nexus IStatePort" |
| ADR-026 | Change "hex-hub resolves secrets" → "hex-nexus resolves secrets" |
| ADR-027 | No change needed (already references hex-nexus) |

---

## Post-Consolidation Architecture

```
┌──────────────────────────────────────────────────┐
│                   hex-nexus                       │
│            (library + binary crate)               │
│                                                   │
│  ┌─────────┐  ┌──────────┐  ┌────────────────┐  │
│  │ Routes  │  │ HexFlo   │  │ Orchestration  │  │
│  │ (54+)   │  │ Coord    │  │ Agent/Workplan │  │
│  └────┬────┘  └────┬─────┘  └───────┬────────┘  │
│       │            │                 │            │
│  ┌────┴────────────┴─────────────────┴────────┐  │
│  │              IStatePort                     │  │
│  │   ┌──────────────┐  ┌───────────────────┐  │  │
│  │   │ SQLite       │  │ SpacetimeDB       │  │  │
│  │   │ (default)    │  │ (feature-gated)   │  │  │
│  │   └──────────────┘  └───────────────────┘  │  │
│  └─────────────────────────────────────────────┘  │
│                                                   │
│  ┌───────┐  ┌──────┐  ┌────────┐  ┌──────────┐  │
│  │  RL   │  │Fleet │  │Secrets │  │Dashboard │  │
│  │Engine │  │Mgmt  │  │Broker  │  │(embedded)│  │
│  └───────┘  └──────┘  └────────┘  └──────────┘  │
│                                                   │
│  bin/hex-nexus.rs  ← 57 LOC entry point          │
└──────────────┬───────────────────────────────────┘
               │ WebSocket
               ▼
┌──────────────────────────────────────────────────┐
│                   hex-agent                       │
│              (independent binary)                 │
│                                                   │
│  Conversation Loop → Tool Execution → Metrics     │
│  Multi-Provider (Anthropic, MiniMax, Ollama)      │
│  RL Model Selection → Prompt Caching              │
│  Secret Claim (env or nexus) → Auto-Compaction    │
└──────────────────────────────────────────────────┘
```

---

## Effort Estimate

| Phase | Files Changed | Risk | Blocking? |
|-------|--------------|------|-----------|
| Phase 1: Move IStatePort | ~8 files | Low | No |
| Phase 2: Add bin target | ~4 files | Low | No |
| Phase 3: Remove hex-hub | ~15 files (docs + configs) | Medium (backward compat) | No |
| Phase 4: Update ADRs | ~6 ADR files | Trivial | No |
| **Total** | **~33 files** | **Low-Medium** | |

Estimated effort: **1-2 sessions** for implementation + testing.

---

## Decision Outcome

Deprecate `hex-hub/` and consolidate into `hex-nexus` (library + binary). hex-agent remains independent. This eliminates a confusing layer of indirection, reunites the state port with its consumers, and simplifies builds. The migration is low-risk because hex-hub contains no unique logic — it is purely structural.

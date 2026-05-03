# ADR-2604261430 — Stash-Backed Consolidation Memory Port

**Status:** Proposed
**Date:** 2026-04-26
**Drivers:** Long-running hex agent sessions accumulate episodic context (chat turns, tool results, ADR drafts, workplan outcomes) but the existing memory surface is a flat key/value store with fuzzy text search. Agents have no structured way to recall facts, detect contradictions across sessions, trace causal chains between failures and goals, or decay stale beliefs. Re-implementing that machinery in `hex-nexus` would duplicate work already done — and done well — by an upstream Apache-2.0 project.
**Related:** ADR-025 (state port consolidation), ADR-2604051800 (memory scope capability checks), ADR-2604112000 (memory port split, P5), ADR-2604120202 (tiered inference routing), ADR-2604131630 (T2.5 reasoning tier)

## Context

Hex's current memory surface is `IHexFloMemoryStatePort` (`hex-nexus/src/ports/state.rs:634`) plus the legacy `ICoordinationPort` memory methods (`hex-core/src/ports/coordination.rs:90`). Both expose the same shape:

```rust
async fn hexflo_memory_store(&self, key: &str, value: &str, scope: &str) -> Result<(), StateError>;
async fn hexflo_memory_retrieve(&self, key: &str) -> Result<Option<String>, StateError>;
async fn hexflo_memory_search(&self, query: &str) -> Result<Vec<(String, String)>, StateError>;
async fn hexflo_memory_delete(&self, key: &str) -> Result<(), StateError>;
```

This is the right surface for swarm coordination — handing a workplan ID between agents, parking a heartbeat token, sharing a partial result. It is the **wrong** surface for cross-session knowledge. Agents need to ask:

- "What facts do I believe about this codebase, and how confident am I?"
- "Which of my prior beliefs contradict each other?"
- "Which goals have I abandoned, and what failures preceded them?"
- "Which hypotheses am I still verifying, and what evidence have I gathered?"

None of that fits a flat KV/fuzzy-search interface. We have three options:

1. **Build it ourselves** in `hex-nexus/src/coordination/`. Months of work. Owns the embedding stack, contradiction detection, decay scheduling, etc.
2. **Bolt a vector DB on the side** (e.g. AgentDB, already referenced in repo skills). Solves search but not the consolidation pipeline — we'd still need to write the 8 stages.
3. **Adopt an upstream consolidation engine behind a port.** Pay the integration cost once; track upstream improvements.

Stash (`github.com/alash3al/stash`, Apache-2.0, pinned at commit `d1122a699cf2f0022409fbdf97871298273c20a6` dated 2026-04-25) is option 3. It implements an 8-stage consolidation pipeline on Postgres + pgvector:

| Stage | Output |
|---|---|
| 1. Episodes → Facts | Cluster + dedupe episodes into confidence-scored facts |
| 2. Facts → Relationships | Extract entity edges |
| 3. Facts + Relationships → Patterns | Higher-level abstractions |
| 4. Contradiction detection | Flag + auto-resolve conflicting facts |
| 5. Confidence decay | Age out stale facts |
| 6. Goal progress inference | Annotate / auto-complete goals |
| 7. Failure pattern detection | Extract recurring failure shapes |
| 8. Hypothesis evidence scanning | Auto-confirm/reject vs. accumulated evidence |

Stash speaks MCP-over-SSE and exposes 28 tools across episodic I/O, knowledge queries, contradictions, causal graphs, hypotheses, goals, and failures (`cmd/cli/mcp.go`). Its consolidation worker is a goroutine driven by a configurable interval (`cmd/cli/serve_all.go`, `internal/brain/consolidate.go`). The reasoner is a single OpenAI-compatible client (`internal/reasoner/openai.go`); pointing `STASH_OPENAI_BASE_URL` at hex's `inference-gateway` routes its consolidation calls through hex's tier-aware dispatch.

Hex's CLAUDE.md already commits to "model-agnostic" inference and a tiered routing strategy. Stash's reasoner-via-base-URL design fits that contract without modification.

## Decision

Add a new secondary port `IConsolidationMemoryPort` and ship a Stash-backed adapter. Keep the existing `IHexFloMemoryStatePort` as-is for coordination KV.

### New port

`hex-core/src/ports/consolidation_memory.rs`. Trait surface (narrowed from stash's 28 MCP tools to the high-value subset; the rest can be added later):

```rust
#[async_trait]
pub trait IConsolidationMemoryPort: Send + Sync {
    // Episodic I/O
    async fn remember(&self, content: &str, namespace: &str) -> Result<EpisodeId, ConsolidationError>;
    async fn recall(&self, query: &str, namespaces: &[String], limit: u32) -> Result<Vec<Episode>, ConsolidationError>;
    async fn forget(&self, about: &str, namespaces: &[String]) -> Result<u32, ConsolidationError>;

    // Trigger consolidation (idempotent; only processes new data since last run)
    async fn consolidate(&self, namespaces: &[String]) -> Result<ConsolidationReport, ConsolidationError>;

    // Knowledge queries
    async fn query_facts(&self, query: &str, namespace: &str) -> Result<Vec<Fact>, ConsolidationError>;
    async fn query_relationships(&self, entity: &str, namespace: &str) -> Result<Vec<Relationship>, ConsolidationError>;

    // Contradictions
    async fn list_contradictions(&self, namespace: &str) -> Result<Vec<Contradiction>, ConsolidationError>;
    async fn resolve_contradiction(&self, id: &str, resolution: &str) -> Result<(), ConsolidationError>;

    // Causal graph
    async fn trace_causal_chain(&self, fact_id: &str, direction: CausalDirection, max_depth: u8) -> Result<CausalChain, ConsolidationError>;

    // Hypotheses / goals / failures (rounded out in Phase 2)
}
```

Domain types (`Episode`, `Fact`, `Relationship`, `Contradiction`, `CausalChain`, `ConsolidationReport`) live in `hex-core/src/domain/consolidation.rs`. Per ADR-001, ports import only domain — no Postgres or pgvector types leak across.

### Adapters

Two adapters, both in `hex-nexus/src/adapters/`:

1. **`StashSseAdapter`** — production path. Spawns or attaches to a stash sidecar (binary or container) and proxies the port methods to its MCP-over-SSE endpoint. The sidecar reads `STASH_POSTGRES_DSN` and `STASH_OPENAI_BASE_URL=http://127.0.0.1:5555/api/inference/openai-compat`, sending consolidation reasoner calls through hex's `inference-gateway`.
2. **`NoopConsolidationAdapter`** — degrade-gracefully default. `remember` and `recall` log + return empty/typed-zero. Selected when `STASH_POSTGRES_DSN` is unset, so single-binary deployments aren't forced to run Postgres.

Adapter selection happens in `composition-root.ts` (TypeScript library) and the equivalent Rust composition in `hex-nexus/src/main.rs`. Both follow ADR-001 rule 6: only the composition root touches adapter constructors.

### Inference routing

Stash's reasoner reads `STASH_REASONER_MODEL`. We register the OpenAI-compatible facade in `inference-gateway` so that:

- `STASH_REASONER_MODEL=hex/t2.5` resolves to the configured T2.5 model (`devstral-small-2:24b` by default, per ADR-2604120202).
- `STASH_EMBEDDING_MODEL=hex/embed` resolves to whichever embedding model the inference-gateway exposes (configurable in `.hex/project.json`).

Tier choice for consolidation defaults to T2.5 because the pipeline performs cross-fact reasoning, not single-shot transforms. Override per-namespace in `.hex/project.json` → `consolidation.tier_models`.

### CLI surface

Additive to `hex memory`:

```
hex memory consolidate [--namespace <ns>...]
hex memory recall <query> [--namespace <ns>...] [--limit N]
hex memory facts <query> [--namespace <ns>]
hex memory contradictions [--namespace <ns>] [--resolve <id> --with <text>]
hex memory causal-trace <fact-id> [--direction forward|backward] [--depth N]
```

Existing `hex memory store|get|search|delete` keep their current semantics and target `IHexFloMemoryStatePort`. The two surfaces cohabit.

### MCP surface

Add `mcp__hex__hex_memory_*` tools (note the `hex_memory_` prefix vs. the existing `hex_hexflo_memory_` prefix — the namespace difference is intentional and signals "consolidation memory" vs "coordination KV"). Schemas in `hex-cli/assets/mcp/mcp-tools.json` mirror the new CLI subcommands 1:1.

### Deployment

- `hex nexus start` learns to start a stash sidecar when `consolidation.enabled` is `true` in `.hex/project.json`. The sidecar is supervised the same way the SpacetimeDB process is.
- `docker-compose.yml` in `examples/` gains a `stash` + `postgres` service for full-stack examples.
- `hex doctor consolidation` checks: Postgres reachable, stash sidecar healthy, `inference-gateway` proxy responding to a test `embeddings` call, port impl wired into composition root.

## Consequences

**Positive:**
- Hex gains a structured semantic memory layer in adapter-time, not engineer-quarters time.
- Reuses upstream improvements automatically (stash is actively maintained as of the pinned commit).
- Inference cost stays under hex's tier governance — stash's reasoner calls inherit our routing, telemetry, and rate limiting.
- The Noop adapter preserves the "single-binary, no Postgres" deployment story for users who don't need consolidation.
- Two cohabiting memory surfaces are explicit, not accidental: KV for coordination state, consolidation for cross-session knowledge.

**Negative:**
- New runtime dependency: Postgres + pgvector. Compose template grows; minimum-machine docs need an update.
- Sidecar process to supervise. Crash-loop, port collision, log forwarding, and version-skew handling are all new failure modes for `hex doctor`.
- Two memory surfaces double the documentation surface. Risk of agents/users picking the wrong one. Mitigate with a "when to use which" table in `docs/specs/memory.md` and a doctor lint that warns when an agent uses `hexflo_memory_store` for content longer than 4 KB (a likely sign they meant `remember`).
- License compliance work: ship `LICENSE-stash` alongside the adapter, preserve copyright notices, and document the dependency in `THIRD_PARTY.md`.
- Embedding-dimension lock-in: `STASH_VECTOR_DIM` defaults to 1536 (matching `text-embedding-3-small`). Switching embedding models later means reindexing.

**Mitigations:**
- Pin the stash sidecar to the commit cited above; bump deliberately via ADR amendment so we observe pipeline behavior changes.
- Sidecar supervision reuses the existing SpacetimeDB supervision path in `hex-nexus/src/orchestration/`.
- Add `hex memory which-store <case>` heuristic command that recommends KV vs. consolidation given a sample payload.
- Require any embedding-model change to ship with a `hex memory reindex` migration.

## Implementation

| Phase | Description | Status |
|-------|------------|--------|
| P1 | Domain types in `hex-core/src/domain/consolidation.rs` and trait in `hex-core/src/ports/consolidation_memory.rs`; no impls yet | Pending |
| P2 | `NoopConsolidationAdapter` in `hex-nexus/src/adapters/`; wire into composition root behind a feature flag default-on | Pending |
| P3 | OpenAI-compat facade for `inference-gateway` so `STASH_OPENAI_BASE_URL` can target it; covers `chat/completions` + `embeddings` | Pending |
| P4 | `StashSseAdapter` proxying the port to stash's MCP-over-SSE; sidecar supervision in `hex-nexus/src/orchestration/` | Pending |
| P5 | CLI subcommands (`hex memory consolidate|recall|facts|contradictions|causal-trace`) + MCP tool definitions | Pending |
| P6 | `hex doctor consolidation` health check; smoke test in `tests/smoke/consolidation_memory.rs` (skipped unless `STASH_POSTGRES_DSN` is set) | Pending |
| P7 | `examples/` Docker Compose update with `stash` + `postgres`; `docs/specs/memory.md` "when to use which" guide; `THIRD_PARTY.md` entry | Pending |
| P8 | Phase 2 port surface: hypotheses, goals, failures, patterns — separate ADR amendment once P1–P7 land | Pending |

## Citation

This ADR adopts the consolidation pipeline design from the Stash project:

> **Stash — Persistent memory for AI agents**
> Author: alash3al (https://github.com/alash3al)
> Repository: https://github.com/alash3al/stash
> License: Apache-2.0
> Pinned commit: `d1122a699cf2f0022409fbdf97871298273c20a6` (2026-04-25)
> Pipeline reference: `internal/brain/consolidate.go`
> MCP surface reference: `cmd/cli/mcp.go`

The 8-stage consolidation pipeline summarized in the Context section, the MCP tool taxonomy reflected in the port surface, and the reasoner-via-base-URL integration pattern are all attributable to the Stash project. The hex-side adapter MUST ship `LICENSE-stash` (verbatim Apache-2.0) and a SOURCE.md noting the upstream commit, per Apache-2.0 §4.

## References

- `hex-core/src/ports/coordination.rs:90` — legacy memory methods on `ICoordinationPort`
- `hex-nexus/src/ports/state.rs:634` — current `IHexFloMemoryStatePort`
- `hex-nexus/src/coordination/memory.rs` — current `MemoryEntry` data model
- `hex-nexus/src/routes/hexflo.rs:25-142` — current memory REST endpoints
- `hex-cli/src/commands/memory.rs` — current CLI surface
- ADR-025 — state port consolidation
- ADR-2604051800 — memory scope capability checks (P1)
- ADR-2604112000 — memory port split (P5)
- ADR-2604120202, ADR-2604131630 — tiered inference routing (T1/T2/T2.5/T3)

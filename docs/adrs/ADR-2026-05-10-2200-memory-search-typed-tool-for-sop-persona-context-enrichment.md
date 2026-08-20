# ADR-2026-05-10-2200 — memory_search typed tool for SOP persona context enrichment

Status: **Proposed**
Date: 2026-05-10

ADR-2026-05-10-2200 — memory_search typed tool for SOP persona context enrichment

**Status**: **Proposed**  
**Related**: ADR-[PHONE] (typed tool library), ADR-[PHONE] (SOP executor), ADR-[PHONE] (materialization gap fix — smoking-gun incident)

## Context

**Smoking gun (2026-05-10):** CTO persona drafted ADR-[PHONE] using the **obsolete [PERSON_NAME] timestamp naming scheme** (`ADR-[PHONE]`) and **cross-referenced it incorrectly** as `ADR-[PHONE]` in the body — even though `hexflo_memory` had contained `lesson:ADR-id-format-must-avoid-phone-pattern` since 2026-05-09 with the agreed **`YYYY-MMDD-slug` scheme** and the three exact code touch points (adr_draft.rs, repo_read, docs/ conventions).

**Root cause:** The SOP executor (ADR-[PHONE]) has **no typed tool to query `hexflo_memory`** during the GROUND phase. Personas receive:
1. `prefetched_paths` — explicit file reads from the operator message
2. `repo_grep` results — keyword pattern match across the codebase

But **zero access to memory** — the database where operators, swarms, and other personas store lessons, decisions, and configuration. The lesson existed in STDB; the CTO persona couldn't see it.

**Existing infrastructure (already works):**
- `hexflo_memory` STDB table (spacetime-modules/hexflo-coordination/src/lib.rs) — key-value store with scope
- `hexflo_memory_search(query: &str)` reducer — substring backend (case-insensitive scan of key + value)
- `IStatePort::hexflo_memory_search(&self, query: &str)` port trait — `hex-nexus/src/ports/state.rs:718`
- `SpacetimeStateAdapter::hexflo_memory_search` impl — `hex-nexus/src/adapters/spacetime_state.rs:1164` — full table scan + client-side filter

**Gap:** No typed tool wraps `hexflo_memory_search` for LLM function-calling. Personas can't auto-retrieve prior art during GROUND.

**Operator priority:** unblocks ADR regression class (personas rediscovering decisions already in memory). Vector backend (AgentDB / ruvector) is separate later work; substring is the cheap unblock.

## Decision

**Ship `memory_search` as a first-class typed SOP tool** with automatic injection into the GROUND phase for SOP-enabled personas.

### 1. New tool: `hex-nexus/src/tools/memory_search.rs`

```rust
//! `memory_search` — query hexflo_memory for lessons, decisions, config.
//!
//! Used by SOP personas during GROUND phase to surface prior art before
//! reasoning. Returns key-value pairs from hexflo_memory (substring
//! backend; case-insensitive scan). Vector search (AgentDB/ruvector) is
//! a future upgrade — this unblocks ADR regression TODAY.

use async_trait::async_trait;
use serde_json::{json, Value};
use std::time::Instant;

use super::{Tool, ToolResult};
use crate::ports::state::IStatePort;
use crate::state::AppState;

pub struct MemorySearch {
    state: std::sync::Arc<AppState>,
}

impl MemorySearch {
    pub fn new(state: std::sync::Arc<AppState>) -> Self {
        Self { state }
    }
}

#[async_trait]
impl Tool for MemorySearch {
    fn name(&self) -> &'static str {
        "memory_search"
    }
    fn description(&self) -> &'static str {
        "Search hexflo_memory for lessons, decisions, and prior art. \
         Returns key-value pairs where key or value contains the query \
         (case-insensitive substring match). Use this during GROUND to \
         check for existing decisions/lessons before drafting new ones. \
         Returns at most k results (default 5)."
    }
    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Search term (substring, case-insensitive). Example: 'ADR-id-format', 'lesson:', 'workplan'.",
                },
                "k": {
                    "type": "integer",
                    "description": "Max results to return. Default 5, cap 20.",
                }
            },
            "required": ["query"]
        })
    }
    async fn execute(&self, input: Value) -> ToolResult {
        let start = Instant::now();
        let query = match input.get("query").and_then(|v| v.as_str()) {
            Some(q) if !q.is_empty() => q.to_string(),
            _ => return ToolResult::err("missing or empty `query`", start.elapsed().as_millis() as u64),
        };
        let k = input.get("k").and_then(|v| v.as_u64()).unwrap_or(5) as usize;
        let k = k.min(20); // hard cap

        let sp = &self.state.stdb;
        let entries = match sp.hexflo_memory_search(&query).await {
            Ok(e) => e,
            Err(e) => {
                return ToolResult::err(
                    format!("hexflo_memory_search failed: {}", e),
                    start.elapsed().as_millis() as u64,
                );
            }
        };

        let truncated = entries.len() > k;
        let results: Vec<Value> = entries
            .into_iter()
            .take(k)
            .map(|(key, value)| json!({ "key": key, "value": value }))
            .collect();

        let elapsed = start.elapsed().as_millis() as u64;
        let output = json!({
            "results": results,
            "total": results.len(),
            "truncated": truncated,
            "query": query,
        });
        if truncated {
            ToolResult::ok_truncated(output, elapsed)
        } else {
            ToolResult::ok(output, elapsed)
        }
    }
}
```

### 2. Register in `hex-nexus/src/tools/mod.rs`

Add to `pub mod` block:
```rust
pub mod memory_search;
```

Add to `ToolRegistry::default()`:
```rust
impl Default for ToolRegistry {
    fn default() -> Self {
        let mut reg = Self::new();
        reg.register(Arc::new(cargo_check::CargoCheck));
        reg.register(Arc::new(repo_grep::RepoGrep));
        reg.register(Arc::new(repo_read::RepoRead));
        reg.register(Arc::new(web_search::WebSearch));
        reg.register(Arc::new(adr_draft::AdrDraft));
        reg.register(Arc::new(spec_draft::SpecDraft));
        reg.register(Arc::new(code_patch::CodePatch));
        reg.register(Arc::new(workplan_emit::WorkplanEmit));
        reg.register(Arc::new(adr_status_set::AdrStatusSet));
        reg.register(Arc::new(escalate_to_operator::EscalateToOperator));
        reg.register(Arc::new(memory_search::MemorySearch::new(/* AppState wiring TBD */)));
        reg
    }
}
```

**Constructor wiring challenge:** `MemorySearch` needs `Arc<AppState>` but `ToolRegistry::default()` is static. **Resolution:** pass `Arc<AppState>` into the SOP executor, construct `MemorySearch` tool on-the-fly in `reason_with_tools()` and inject it into the registry clone. See P1.2 below.

### 3. Auto-inject into GROUND phase (`hex-nexus/src/orchestration/sop_executor.rs`)

**Insertion point:** `ground_for_intent()` function, after the `repo_grep` call, before returning the ground_pack JSON.

**Logic:**
```rust
async fn ground_for_intent(
    registry: &Arc<ToolRegistry>,
    intent: &str,
    operator_message: &str,
) -> Value {
    // ... existing prefetched_paths logic ...
    // ... existing repo_grep logic ...

    // NEW: auto-call memory_search with query = distinctive keywords from operator message
    let memory_query = derive_grep_pattern(operator_message); // reuse same stopword filter
    let memory_input = json!({ "query": memory_query, "k": 5 });
    let memory_result = registry.execute("memory_search", memory_input).await;

    json!({
        "intent": intent,
        "prefetched_paths": prefetched,
        "repo_grep": grep_result.output,
        "memory_search": memory_result.output,  // <-- NEW
    })
}
```

**Effect:** Every SOP persona sees `ground_pack.memory_search.results: [{ key, value }, ...]` in their context **before REASON**, eliminating the "I can't see lessons" gap.

### 4. Update `ToolRegistry` to accept stateful tools

**Problem:** Current `ToolRegistry::default()` is static; `MemorySearch` needs `Arc<AppState>`.

**Solution (minimal disruption):**

Change `hex-nexus/src/orchestration/sop_executor.rs` to pass `Arc<AppState>` into `run()`:
```rust
pub async fn run(
    role: &str,
    operator_message: &str,
    repo_root: &str,
    state: Arc<AppState>,  // <-- NEW
) -> SopResult {
    // ... existing CLASSIFY ...
    // ... existing GROUND ...

    // PHASE 3 REASON — clone registry and inject MemorySearch with state
    let mut registry = ToolRegistry::default();
    registry.register(Arc::new(memory_search::MemorySearch::new(state.clone())));
    let registry = Arc::new(registry);

    let reason_result = reason_with_tools(role, operator_message, intent, &ground_pack, registry.clone()).await;
    // ... existing VERIFY + EMIT ...
}
```

Caller (org_responder) already has `Arc<AppState>` in scope — pass it through.

## Consequences

### ✅ Positive

1. **Regression class closed:** Personas will see `lesson:ADR-id-format-must-avoid-phone-pattern` (and all other memory entries) in the GROUND pack before drafting ADRs, eliminating rediscovery of decisions already in memory.

2. **Zero LLM cost:** `memory_search` is deterministic (substring scan); no additional inference tokens.

3. **Automatic:** Personas don't need to explicitly invoke `memory_search` — SOP executor injects it into GROUND for every ask, using the same keyword extraction already done for `repo_grep`.

4. **Immediate unblock:** Substring backend (already shipping in `hexflo_memory_search` reducer) is Good Enough™ for the lesson/decision use case. Vector search (AgentDB / ruvector) can land later without changing the tool API.

5. **Observable:** `memory_search` results appear in SOP trace → operator can see "GROUND → memory_search returned N hits" in chat cards and debug logs.

### ⚠️ Risks

1. **`AppState` plumbing:** `ToolRegistry` becomes stateful (needs `Arc<AppState>` to construct `MemorySearch`). Mitigated by injecting `MemorySearch` late (in `sop_executor::run()`) rather than modifying `ToolRegistry::default()`.

2. **GROUND bloat:** If `hexflo_memory` grows large (>1000 entries), substring scan + keyword match may return noise. Mitigated by:
   - `k=5` default cap (only top 5 matches injected)
   - Future: vector search + semantic ranking (separate ADR)

3. **Scope blind:** Current `hexflo_memory_search` port trait has no `scope` parameter (see `spacetime_state.rs:1167` comment). All scopes ("global", "swarm:*", "agent:*") are scanned. Acceptable for now — lessons are typically global-scoped.

## Verification

**Done-condition (ADR-[PHONE] §4):**

1. `cargo check --workspace` passes
2. `hex-nexus/src/tools/mod.rs` lists `memory_search` in `ToolRegistry::default()` OR `sop_executor::run()` injects it
3. Sending `@cto draft an ADR about <topic>` triggers a SOP run trace showing:
   ```
   GROUND → memory_search(query="topic", k=5) returned N results
   ```
4. A follow-up `@cto` ADR draft uses the new `2026-MMDD-slug` naming scheme (evidencing the lesson was visible)

**Acceptance test:**

```bash
# 1. Insert test lesson into hexflo_memory via hex CLI or reducer call
spacetime-cli call hex_hexflo_memory_store '["lesson:test-memory-search", "This is a test lesson for memory_search tool verification", "global", "2026-05-10T22:00:00Z"]'

# 2. Send @cto a message mentioning "test memory search"
# Expected: SOP trace shows memory_search hit

# 3. Confirm lesson content appears in CTO's REASON context
# Expected: CTO references the lesson in response
```

---

**This ADR unblocks the smoking-gun regression class (personas rediscovering decisions) by wiring `hexflo_memory` into the SOP GROUND phase as a first-class typed tool. Substring backend ships today; vector search is a future upgrade with zero tool-API change.**

# ADR-2026-05-09-2145 — Tool Czar persona for toolchain health monitoring

Status: **Proposed**
Date: 2026-05-09

ADR-2026-05-09-2145: Tool Czar persona for toolchain health monitoring

**Status:** **Proposed**  
**Deciders:** CTO, CPO, operator  
**Date:** 2026-05-09

## Context

Recurring pattern identified 2026-05-09: **tool failures go undetected until a persona stumbles into them**, causing wasted inference cycles, persona confusion, and delayed ADR delivery. Five documented incidents in `lesson:need-tool-czar-persona`:

1. **rg binary missing** (`lesson:rg-binary-required`) — `repo_grep` silently returned empty results when ripgrep wasn't in PATH; personas assumed "no matches" was correct ground. Cost: 3 rounds of re-grounding.
2. **[PERSON_NAME] content-filter blocks** (`lesson:openrouter-content-filter-blocks-security`) — security personas (CISO, adversarial-red) hit HTTP 403 on inference requests containing `secret`, `exploit`, `vulnerability`; the fallback path to Ollama was added (ADR-[PHONE]) but the initial round trip still wastes 4-8s per REASON phase.
3. **auto-emitter deduplication bug** (`lesson:auto-emitter-dedup-bug`) — stale hash cache caused duplicate ADRs to be silently dropped; operator discovered it 48h later after noticing missing ADR files despite successful tool calls.
4. **web_search API key rotation** — [PERSON_NAME] key expired; `web_search` returned `ToolResult::err("401 Unauthorized")` but no alerting existed; personas wasted rounds proposing alternative approaches instead of escalating the infrastructure gap.
5. **cargo_check timeout on large workspace** — 90s hard timeout in `cargo_check` tool caused false negatives on fresh builds; personas interpreted timeout as "build broken" and proposed unnecessary reverts.

**Current gap:** No proactive toolchain health monitoring. We rely on personas to detect + escalate tool failures, which:
- Wastes inference budget (tier-3/4 models spinning on infrastructure problems)
- Delays ADR delivery (re-grounding loops)
- Erodes trust (personas start second-guessing tool outputs)

**Precedent:** `resource_supervisor` (ADR-[PHONE]) runs scheduled 60s ticks to monitor process health (RSS, CPU%, zombies). We need the same pattern for **typed tools**.

---

## Decision

### 1. NEW PERSONA: `tool-czar`

Create `hex-cli/assets/agents/hex/hex/tool-czar.yml`:

```yaml
name: tool-czar
role: Tool Czar
description: |
  Toolchain health monitor. Probes each typed tool every 15 minutes with benign
  payloads, tracks success rate + latency + error patterns, escalates degradations
  to CTO, drafts gap ADRs for systemic failures.

tier: executive
reports_to: cto

responsibilities:
  - Smoke-test all typed tools on 15-minute tick
  - Monitor rolling 24h success rate per (tool, persona)
  - Escalate <80% success or >5min hard-down to operator
  - Draft ADRs for systemic tool gaps (missing deps, API key rot, etc.)
  - Maintain dashboard at #/tools with per-tool traffic lights

model:
  preferred: qwen3:4b          # Cheap for probes
  escalation: claude-opus-4-7  # ADR authoring only
  fallback: qwen2.5-coder:32b
  upgrade_threshold: 0.9

context_level: L1  # Tools + STDB schema only

workflow:
  phases:
    - name: probe
      description: Run benign smoke tests on all tools
    - name: observe
      description: Record success/latency/error to STDB
    - name: analyze
      description: Query rolling 24h success rate per tool
    - name: escalate
      description: Draft ADR or notify operator on degradation

delegation:
  can_spawn: []
  must_consult:
    - cto  # Escalation threshold tuning
```

### 2. NEW STDB TABLES (spacetime-modules/hexflo-coordination/src/lib.rs)

```rust
/// Per-probe observation — records every tool health check attempt.
#[table(name = tool_health_observation, public)]
#[derive(Clone, Debug)]
pub struct ToolHealthObservation {
    #[primary_key]
    #[auto_inc]
    pub id: u64,
    pub tool_name: String,
    /// Persona that invoked the tool (empty for czar probes)
    pub persona: String,
    pub ts: String,
    pub success: bool,
    pub latency_ms: i32,
    pub error_msg: String,
}

/// Aggregated 24h health view — computed by tool_czar_supervisor_tick.
/// Replaces this table each tick with fresh rolling stats.
#[table(name = tool_health_summary, public)]
#[derive(Clone, Debug)]
pub struct ToolHealthSummary {
    #[unique]
    pub tool_persona_key: String,  // "{tool_name}::{persona}"
    pub tool_name: String,
    pub persona: String,
    pub rolling_24h_success_rate: f32,
    pub rolling_24h_count: u32,
    pub last_failure_ts: String,
    pub last_failure_msg: String,
}
```

**Reducers:**
- `tool_health_observe(tool_name, persona, ts, success, latency_ms, error_msg)` — insert row
- `tool_health_summary_recompute()` — called by supervisor tick; truncates + rebuilds summary table from last 24h of observations

### 3. NEW DAEMON: `tool_czar_supervisor` (hex-nexus/src/orchestration/tool_czar_supervisor.rs)

**15-minute scheduled tick** (similar to `resource_supervisor_tick` pattern):

```rust
//! Tool Czar supervisor (ADR-2026-05-09-2145).
//!
//! Spawns a tokio task that:
//! 1. Every 15 minutes, probes each typed tool with a benign payload
//! 2. Writes tool_health_observation rows to STDB
//! 3. Computes rolling 24h success rate per (tool, persona="__czar__")
//! 4. Escalates <80% success via inbox notification or ADR draft
//!
//! Probe payloads designed to be side-effect-free + fast:
//! - cargo_check: empty string (should fail gracefully)
//! - repo_grep: `pub fn` (always matches in Rust repo)
//! - repo_read: README.md offset=1 limit=10
//! - web_search: "rust ripgrep" if TAVILY_API_KEY present
//! - escalate_to_operator: dry-run flag (no actual notification)
//! - adr_draft/spec_draft/workplan_emit: skip (write tools, czar can't test without side effects)

pub fn spawn(stdb_host: String, hex_db: String, tool_registry: Arc<ToolRegistry>);
```

**Implementation notes:**
- Reuses `ToolRegistry::execute()` from `hex-nexus/src/tools/mod.rs`
- Writes observations via `/v1/database/{hex_db}/call/tool_health_observe`
- Computes summary every tick via `/v1/database/{hex_db}/call/tool_health_summary_recompute`
- Skips write-tools (`adr_draft`, `code_patch`, `workplan_emit`) to avoid side effects

### 4. PROBE SET for system dependencies

Tool czar's `probe` phase checks:

| Tool            | Probe payload                                   | Pass condition                     |
|-----------------|-------------------------------------------------|------------------------------------|
| `cargo_check`   | `{ "crate": "" }`                               | `ok: false` (graceful empty crate) |
| `repo_grep`     | `{ "pattern": "pub fn" }`                       | `ok: true`, matches > 0            |
| `repo_read`     | `{ "path": "README.md", "limit": 10 }`          | `ok: true`, returned_lines > 0     |
| `web_search`    | `{ "query": "rust ripgrep" }` (if key present)  | `ok: true` OR `error: "no API key"`|
| `escalate_to_operator` | `{ "reason": "[PERSON_NAME]", "urgency": "low" }` (dry-run) | `ok: true`                         |

**External deps probed:**
- `which rg` → writes observation with `tool_name="rg_binary"`, `success=(exit==0)`
- `cargo --version` → writes `tool_name="cargo_binary"`
- `ollama list` → writes `tool_name="ollama_binary"`
- `env | grep -E '(ANTHROPIC|OPENROUTER|TAVILY)_API_KEY'` → writes one observation per key

### 5. ESCALATION POLICY

| Condition                                  | Action                                                   |
|--------------------------------------------|----------------------------------------------------------|
| Tool <80% rolling 24h success              | tool-czar drafts ADR via `adr_draft` naming the gap      |
| Tool hard-down >5min (0% success, ≥3 obs)  | `escalate_to_operator(urgency=high, reason=<tool down>)` |
| External dep missing (rg/cargo/ollama)     | `escalate_to_operator(urgency=med, reason=<dep missing>)`|
| API key missing/expired (web_search 401)   | `escalate_to_operator(urgency=med, reason=<key invalid>)`|

**ADR drafting logic:**
- tool-czar switches to `claude-opus-4-7` (escalation model) for ADR authoring
- Includes: last 10 error messages, affected personas, recommended mitigation
- Example: `ADR-<ts>: repo_grep reliability — rg binary missing on 3/5 nodes`

### 6. DASHBOARD: new route `#/tools`

**hex-nexus/assets/src/components/views/ToolHealth.tsx** (React):

- **Per-tool traffic-light grid:** green (≥95%), yellow (80-95%), red (<80%)
- **Per-persona heatmap:** rows=tools, cols=personas, cell color=success rate
- **Recent failures table:** last 20 `tool_health_observation` rows where `success=false`
- **Probe history sparkline:** 7-day success rate trend per tool

**Route wiring:** `hex-nexus/src/routes.rs` + `ToolHealth` component in nexus assets

### 7. MIGRATION + BACKFILL

**Phase 1:** Seed initial gap log from today's lessons:
- `tool_health_observation` insert for `rg_binary` failure (ts=<incident time>, success=false, error_msg="binary not found in PATH")
- `tool_health_observation` insert for `web_search` API key rot (ts=<incident time>, success=false, error_msg="401 Unauthorized")
- `tool_health_observation` insert for `auto_emitter` dedup bug (ts=<incident time>, success=false, error_msg="duplicate ADR dropped by stale hash")

**Phase 2:** tool-czar runs first probe cycle, establishes baseline

**Phase 3:** operator reviews dashboard after 24h, tunes escalation thresholds if needed

---

## Consequences

### Positive

- **Proactive failure detection:** Tool gaps surface within 15 minutes instead of randomly during persona work
- **Reduced inference waste:** Personas no longer spin on infrastructure problems (saves tier-3/4 model budget)
- **Operator visibility:** Dashboard shows toolchain health at a glance; no need to grep logs for tool errors
- **Self-healing:** tool-czar can draft mitigation ADRs autonomously (e.g., "add rg to Docker image")
- **Historical data:** 24h rolling observations enable trend analysis + capacity planning

### Negative

- **STDB write volume:** ~10 observations per tool per hour = ~2400 rows/day (negligible for STDB, but adds to query surface)
- **False positives:** Benign probe failures (e.g., `web_search` when API key intentionally absent) may trigger spurious escalations
  - **Mitigation:** tool-czar's analyze phase checks error patterns; "no API key" is expected, not escalated
- **Probe side effects:** If probe payloads are misconfigured, czar could spam notifications or draft junk ADRs
  - **Mitigation:** P2 (probes) has gating in workplan; operator reviews probe logic before P3 (persona) goes live

### Risks

- **tool-czar itself fails:** If the supervisor task crashes, no one monitors the monitor
  - **Mitigation:** nexus logs tool-czar tick errors at `warn!` level; operator's existing log monitoring catches supervisor crashes
- **Escalation storms:** If multiple tools degrade simultaneously (e.g., STDB down), tool-czar drafts N ADRs in quick succession
  - **Mitigation:** ADR deduplication (auto-emitter) + czar's `must_consult: [cto]` gate prevents spam

---

## Alternatives Considered

1. **Extend resource_supervisor instead of new persona**  
   ❌ Rejected: resource_supervisor monitors processes, not logical tool health. Mixing concerns would bloat the 60s tick.

2. **Operator manually monitors tool errors via logs**  
   ❌ Rejected: doesn't scale; operator shouldn't grep nexus logs for tool failures when a persona can do it.

3. **Embed health checks in each tool's `execute()`**  
   ❌ Rejected: per-tool probes can't detect cross-cutting issues (API key rot, binary missing). Centralized czar sees the full picture.

4. **Use existing `validation-judge` persona for tool QA**  
   ❌ Rejected: validation-judge operates reactively (reviews PRs/ADRs). tool-czar is proactive + runs on a schedule.

---

## Implementation Notes

- **Cheap model for probes:** `qwen3:4b` (or `qwen2.5-coder:7b` fallback) keeps probe cost <$0.01/day
- **ADR escalation uses opus:** tool-czar switches to `claude-opus-4-7` only when drafting gap ADRs (rare, high-value)
- **Dashboard refresh:** ToolHealth.tsx polls `/api/stdb/query?table=tool_health_summary` every 30s
- **Scheduled reducer anchor:** `tool_czar_supervisor_tick_schedule` table with 15-minute `ScheduleAt::Interval`, mirrors `resource_supervisor_tick_schedule` pattern

---

## References

- ADR-[PHONE]: SOP REASON phase Ollama fallback (content-filter mitigation)
- ADR-[PHONE]: Typed tool library + registry
- ADR-[PHONE]: Resource observer (proc walk + supervisor tick pattern)
- `lesson:need-tool-czar-persona`: 2026-05-09 diagnosis of recurring tool failures
- `lesson:rg-binary-required`, `lesson:openrouter-content-filter-blocks-security`, `lesson:auto-emitter-dedup-bug`

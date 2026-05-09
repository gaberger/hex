# Cost and token efficiency operator spec

*status*: proposed  ·  *date*: 2026-05-09

Cost and token efficiency operator spec

**Status**: proposed  
**Owner**: CPO + CTO (cost surfaces + routing implementation)  
**Date**: 2025-06-05

---

## Context

Every SOP run, twin review, drafter inference, and persona reply burns frontier-tier reasoning tokens. Current cost surfaces:

### Current defaults (observed from prefetched code)

1. **SOP executor** (`hex-nexus/src/orchestration/sop_executor.rs`):
   - REASON phase: `max_tokens=4096` per call
   - Tool round-trip cap: 8 iterations
   - Model: `HEX_SOP_REASON_MODEL` env var, defaults to `claude-sonnet-4-5`
   - Cost exposure: up to ~33K tokens per SOP run (8 round-trips × 4096 tokens)

2. **Twin reviewer** (`hex-nexus/src/orchestration/twin_reviewer.rs`):
   - `TWIN_MAX_TOKENS=512`
   - Every proposed_action triggers one inference call
   - Model: routed via `/api/inference/complete` (tier-selected)

3. **Drafter** (`hex-nexus/src/orchestration/drafter.rs`):
   - `DRAFT_MAX_TOKENS=4096`
   - One call per open commitment
   - Model: routed via `/api/inference/complete`

4. **Tier routing** (`hex-nexus/src/quant_router.rs`):
   - Selects provider by `QuantizationLevel` (Q2/Q4/Q8/Fp16/Cloud)
   - Prefers local (Ollama/vLLM) before cloud
   - Falls back to frontier when lower tiers exhausted

### Representative pricing (approximate, Jan 2025)

| Model                  | Input ($/Mtok) | Output ($/Mtok) | Notes |
|------------------------|----------------|-----------------|-------|
| claude-sonnet-4.5      | $3.00          | $15.00          | Frontier reasoning; extended thinking |
| claude-haiku-4.5       | $0.25          | $1.25           | Fast tier, good for twin/drafter |
| deepseek-v3            | $0.27          | $1.10           | OpenRouter; competitive with Haiku |
| qwen2.5-coder-32b-q4   | $0.00          | $0.00           | Local Ollama; zero cost, higher latency |

**Burn estimate** (one full SOP run, cto persona):
- REASON: 8 round-trips × 4096 tokens = 32,768 output tokens
- Ground pack prefetch: ~16 KB × 3 files = ~12K input tokens
- At Sonnet 4.5 rates: (12K × $3/M) + (33K × $15/M) ≈ **$0.53 per SOP run**
- Monthly burn (100 SOP runs/day): **$1,590/month**

---

## Operator-facing surfaces

### (a) Budget dashboard — daily/monthly token spend tracking

**Location**: Mission Control dashboard, new `/admin/cost-metrics` view

**Metrics displayed**:
- **By persona**: token spend breakdown (cto, cpo, ciso, etc.)
- **By tool**: repo_grep, repo_read, cargo_check token consumption (input tokens from ground pack)
- **By loop**: SOP REASON phase, twin review, drafter calls
- **Cumulative**: daily rolling window, monthly total

**Data source**: 
- STDB `inference_log` table (needs new `input_tokens_billed`, `output_tokens_billed`, `cost_usd` columns)
- Aggregated by `persona`, `tool_name`, `loop_type` dimensions

**UX**:
- Table view: `[Persona | Today $ | MTD $ | Avg $/run]`
- Sparkline: 7-day cost trend per persona
- Alert banner when daily burn > operator-configured threshold (e.g. `HEX_DAILY_COST_LIMIT_USD=50`)

---

### (b) Tier-routing affordances — pin persona to local models, escalate only when high-stakes

**Operator control**: new `~/.hex/cost-policy.yml`

```yaml
personas:
  cto:
    sop_reason_tier: haiku  # Use Haiku for routine CTO SOP runs
    escalate_to_sonnet_when:
      - complexity_high       # score_complexity() returns High
      - cargo_check_failure   # Phase 4 verify fails, retry with Sonnet
  cpo:
    sop_reason_tier: local    # qwen2.5-coder:32b via Ollama
    escalate_to_sonnet_when: []  # Never escalate; local-only
  ciso:
    sop_reason_tier: sonnet   # Security-sensitive; always frontier
```

**Implementation touch-points**:
- `sop_executor.rs:reason_via_anthropic()`: read policy, override `HEX_SOP_REASON_MODEL`
- `quant_router.rs:select_provider_task_aware()`: respect persona-level tier floor
- New helper: `cost_policy::load_persona_tier(role: &str) -> QuantizationLevel`

**Behavioral change**:
- CTO routine file-write tasks: $0.03/run (Haiku) instead of $0.53 (Sonnet)
- Complexity escalator: high-stakes (multi-crate refactor, 30+ file dependency) auto-escalates to Sonnet
- Cost savings: ~85% on routine ops, frontier spend reserved for paradigm/architecture decisions

---

### (c) Per-action cost preview before twin approval

**Dashboard widget**: proposed_action detail view shows estimated cost before operator override

**Preview fields**:
- `estimated_input_tokens`: payload JSON length + memory snapshot length
- `estimated_output_tokens`: 512 (twin default)
- `estimated_cost_usd`: (input × model_in_rate) + (output × model_out_rate)
- `model_used`: e.g. `claude-haiku-4.5` or `qwen2.5-coder:32b-local`

**Data source**:
- Twin already loads operator memory (`MEMORY_CAP_BYTES=32KB`)
- Payload preview is `PAYLOAD_PREVIEW_BYTES=4KB`
- Token estimate: `(memory_len + payload_len) / 3.5` (chars-to-tokens heuristic)

**UX change**:
- Pending action card gains new row: `Est. cost: $0.002 (haiku)`
- Operator sees cost BEFORE clicking approve/reject
- High-cost actions (>$0.10) render in amber with warning icon

---

### (d) Cost gates — auto-reject SOP runs > $X without operator override

**Gate enforcement**: `sop_executor.rs:reason_with_tools()` pre-flight check

**Logic**:
1. Before first Anthropic call, estimate worst-case cost:
   - `max_cost = (ground_pack_tokens × in_rate) + (max_tokens × round_trip_cap × out_rate)`
2. If `max_cost > HEX_SOP_COST_GATE_USD` (default `$1.00`):
   - Emit `escalate_to_operator` tool call with reason: `"estimated cost ${max_cost:.2} exceeds gate ${gate}"`
   - SOP does NOT proceed to REASON phase
   - Operator sees inbox notification with override button

**Override flow**:
- Dashboard: operator clicks "Approve high-cost SOP run"
- Sets per-thread override flag in STDB: `sop_run_override(thread_id, approved_by, max_cost_override_usd)`
- Next SOP poll reads override, proceeds with REASON

**Behavioral impact**:
- Protects against runaway costs (e.g., LLM requests 8 round-trips with 30KB context each)
- Operator retains control: can approve one-off expensive runs (e.g., "analyze entire codebase")

---

### (e) Caching opportunities — reduce redundant token spend

**High-impact caches** (from observed code patterns):

1. **Twin re-reading operator memory every call**  
   - Current: `load_operator_memory()` on every `review_one()` — ~32KB × N actions  
   - Opportunity: cache memory snapshot per tick (20s poll interval), reuse for all actions in batch  
   - Savings: 32KB input tokens × (actions_per_tick - 1)  
   - Implementation: move `let memory = load_operator_memory(memory_dir);` outside the `for action in pending` loop in `twin_reviewer.rs:run_one()`

2. **Drafter re-reading same source files across personas**  
   - Current: each persona's SOP GROUND phase calls `repo_read` independently  
   - Opportunity: STDB-backed content cache (TTL 5 min) keyed by `(path, mtime)`  
   - Savings: ~50% input tokens when multiple personas reference same ADRs/specs  
   - Implementation: new `hex-nexus/src/cache/file_content.rs` with LRU eviction, wire into `repo_read` tool

3. **Anthropic prompt caching** (native API feature)  
   - Anthropic's cache API: mark system prompt + ground pack as cacheable, 90% cost reduction on cache hits  
   - Current: not enabled  
   - Opportunity: SOP system prompt (stable) + prefetched file content (changes slowly)  
   - Savings: $0.30/Mtok (cached input) vs $3/Mtok (fresh input)  
   - Implementation: set `cache_control: { type: "ephemeral" }` on system message + ground pack in `sop_executor.rs:reason_via_anthropic()`

**Aggregate savings estimate**: 60-70% input token cost reduction with all three caches active.

---

### (f) Recommended max_tokens reductions where safe

**Current vs. proposed**:

| Component | Current | Proposed | Rationale |
|-----------|---------|----------|-----------|
| Twin reviewer | 512 | 512 | Already minimal; verdict JSON is ~100 tokens |
| Drafter (one-pagers) | 4096 | 2048 | Most specs/ADRs are <2KB; 2048 tokens ≈ 1.4 pages |
| Drafter (multi-pagers) | 4096 | 4096 | Keep for workplans, analysis docs |
| SOP REASON | 4096 | 3072 | Empirical: 95% of tool calls use <2K tokens; 3K covers edge cases |

**Behavioral safeguard**:
- If LLM hits `max_tokens` and truncates mid-JSON, tool validator rejects the action
- Operator sees "truncated response" in phase trace, can override `max_tokens` per persona via env: `HEX_SOP_MAX_TOKENS_CTO=4096`

**Cost impact**:
- Drafter: 50% output token reduction on routine specs = **$0.015/run** (was $0.03)
- SOP REASON: 25% output token cap = **$0.40/run** (was $0.53)
- Monthly savings (100 drafts + 100 SOP runs): **$400/month**

---

## Success criteria

1. **Dashboard visibility**: Operator can answer "how much did hex cost me today?" in <10 seconds
2. **Tier flexibility**: CTO persona runs on local qwen2.5-coder for routine tasks, Sonnet for architecture reviews — without code changes
3. **Cost preview**: Operator sees estimated cost before approving any proposed_action >$0.05
4. **Gate enforcement**: No SOP run exceeds $1 without explicit operator approval
5. **Cache hit rate**: >50% input tokens served from cache after 1 week of runtime (observable via new `cache_hit` metric in `inference_log`)
6. **Measurable burn reduction**: 70% cost reduction on routine ops (non-frontier personas) within 30 days of deployment

---

## Implementation surface (for CTO)

- `hex-nexus/src/orchestration/sop_executor.rs`: cost gate pre-flight, policy-driven tier selection
- `hex-nexus/src/orchestration/twin_reviewer.rs`: move memory load outside action loop (cache)
- `hex-nexus/src/orchestration/drafter.rs`: reduce `DRAFT_MAX_TOKENS` to 2048, add env override
- `hex-nexus/src/quant_router.rs`: add `load_persona_tier()` helper, read `~/.hex/cost-policy.yml`
- `hex-nexus/src/cache/file_content.rs`: new LRU cache for `repo_read` content
- `hex-nexus/assets/src/views/CostMetrics.tsx`: new dashboard view for spend breakdown
- STDB schema: add `input_tokens_billed`, `output_tokens_billed`, `cost_usd` to `inference_log`

---

## Open questions for operator

1. Should cost gates be **per-SOP-run** ($1 default) or **daily cumulative** ($50/day across all personas)?
2. Preferred tier for **board persona** SOP runs — always Sonnet (high-stakes strategy), or Haiku with manual escalation?
3. Should the dashboard expose **per-file cost attribution** (e.g., "ADR-[PHONE].md has been read 47 times this month, costing $X")?


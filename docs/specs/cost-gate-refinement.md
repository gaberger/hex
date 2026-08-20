# Cost Gate Refinement — Pre-flight Preview, Burn-Rate Widget, Cache Visibility

*status*: proposed  ·  *date*: 2026-05-10  
*references*: mission-control-ux-v2.md, cost-and-token-efficiency.md

---

## Overview

This spec defines three operator-facing cost surfaces that refine the gates and metrics described in `cost-and-token-efficiency.md`:

1. **Pre-flight cost preview modal** — shown before SOP runs estimated >$0.50, with token breakdown and approve/cancel/downgrade-tier options.
2. **Burn-rate widget on Mission Control** — live hour/day/week token spend + sparkline (color-coded green <$5, yellow $5–20, red >$20).
3. **Cache hit rate indicator** — displays Anthropic prompt cache effectiveness (percentage, last 24h).

These surfaces give the operator real-time cost control and visibility without leaving Mission Control.

---

## (a) Pre-flight Cost Preview Modal

### Trigger Logic

Before `sop_executor.rs::reason_with_tools()` begins the REASON phase:

1. Estimate worst-case cost:
   - **Input tokens**: `ground_pack_tokens = (prefetched_file_bytes + tool_result_json_bytes) / 3.5` (chars-to-tokens heuristic)
   - **Output tokens**: `max_tokens × round_trip_cap` (default: 4096 × 8 = 32,768)
   - **Rate lookup**: per-model input/output $/Mtok from `quant_router.rs` or env `HEX_MODEL_RATES_JSON`
   - **Total**: `(input × in_rate) + (output × out_rate)`

2. If `total > $0.50` (configurable via `HEX_COST_PREVIEW_THRESHOLD_USD`), emit `proposed_action`:
   - `kind = "sop_cost_preview"`
   - `payload_json` contains: `{ "thread_id", "persona", "ground_tokens", "reason_tokens", "model", "est_cost_usd" }`
   - `rationale = "SOP run estimated at $X.XX; operator approval required"`

3. SOP executor **pauses** until operator responds (approve/reject/downgrade via dashboard).

### Modal UX

When operator opens Mission Control and sees a pending `sop_cost_preview` action, the modal auto-displays:

```
╔═══════════════════════════════════════════════════════════════╗
║  Cost Preview — SOP Run Requires Approval                    ║
╠═══════════════════════════════════════════════════════════════╣
║                                                               ║
║  Persona:        @cto                                         ║
║  Thread:         thr_2605091430_fix_cache_bug                 ║
║  Model:          claude-sonnet-4.5                            ║
║                                                               ║
║  ─────────────────────────────────────────────────────────── ║
║  TOKEN BREAKDOWN (estimated)                                  ║
║                                                               ║
║  GROUND phase (input):                                        ║
║    • Prefetched files:         12,400 tokens                  ║
║    • Tool results (3 calls):    8,200 tokens                  ║
║    • System prompt:             2,100 tokens                  ║
║    Subtotal input:             22,700 tokens  →  $0.068       ║
║                                                               ║
║  REASON phase (output):                                       ║
║    • Max tokens per call:       4,096 tokens                  ║
║    • Round-trip cap:            ×8 iterations                 ║
║    Subtotal output:            32,768 tokens  →  $0.491       ║
║                                                               ║
║  ─────────────────────────────────────────────────────────── ║
║  ESTIMATED TOTAL:  $0.56                                      ║
║  ─────────────────────────────────────────────────────────── ║
║                                                               ║
║  [ Approve ]   [ Downgrade to Haiku ($0.08) ]   [ Cancel ]  ║
║                                                               ║
╚═══════════════════════════════════════════════════════════════╝
```

### Button Behaviors

- **Approve**: POST `/api/sop/approve-cost-override` with `{ thread_id, approved_cost_usd }` → SOP resumes with Sonnet.
- **Downgrade to Haiku**: POST `/api/sop/approve-cost-override` with `{ thread_id, approved_cost_usd, override_model: "claude-haiku-4.5" }` → SOP resumes with cheaper model.
- **Cancel**: POST `/api/proposed-actions/reject` with action ID → SOP aborts, operator sees "Cost limit exceeded; SOP cancelled" in thread log.

### Implementation Hooks

- **Backend**: `hex-nexus/src/orchestration/sop_executor.rs::estimate_sop_cost()` helper
- **STDB**: new `sop_cost_override(thread_id, approved_by, approved_cost_usd, override_model)` table
- **Frontend**: `hex-nexus/assets/src/components/modals/CostPreviewModal.tsx` (Solid component, auto-opens when `proposed_action.kind == "sop_cost_preview"`)

---

## (b) Burn-Rate Widget on Mission Control

### Widget Placement

**Location**: Top-right of Mission Control, above "Pending Decisions" panel (right column, sticky).

**Layout**: 4-column card (matches Persona Health panel width).

### Data Source

Query STDB `inference_log` table:
- **Hour**: `SUM(cost_usd) WHERE ts > NOW() - 3600s`
- **Day**: `SUM(cost_usd) WHERE ts > NOW() - 86400s`
- **Week**: `SUM(cost_usd) WHERE ts > NOW() - 604800s`
- **Sparkline**: 24 hourly buckets, last 24h, visualized as SVG micro-bar chart

### Widget Mockup

```
╔══════════════════════════════════════════╗
║  Token Burn Rate                         ║
╠══════════════════════════════════════════╣
║                                          ║
║  Last Hour:    $2.34   🟢               ║
║  Today:       $12.80   🟡               ║
║  This Week:   $78.50   🟢               ║
║                                          ║
║  ┌────────────────────────────────────┐ ║
║  │ ▂▃▅▇█▇▅▃▂▁▂▃▄▅▆▇▆▅▄▃▂▁▂▃          │ ║ (sparkline: 24h trend)
║  └────────────────────────────────────┘ ║
║  24h ago                           now   ║
║                                          ║
║  [ View Breakdown → ]                   ║
║                                          ║
╚══════════════════════════════════════════╝
```

### Color Logic

- **Green** 🟢: <$5
- **Yellow** 🟡: $5–20
- **Red** 🔴: >$20

Applies independently to hour/day/week rows.

### Behavior

- **Auto-refresh**: Updates every 5s with Mission Control's global refresh.
- **"View Breakdown" button**: Navigates to `/admin/cost-metrics` (full dashboard from cost-and-token-efficiency.md).
- **Sparkline hover**: Shows exact cost for that hour (tooltip via Solid `title` attribute).

### Implementation Files

- **Endpoint**: `hex-nexus/src/routes/mission_control.rs::get_burn_rate()` → returns `{ hour_usd, day_usd, week_usd, hourly_buckets: [f64; 24] }`
- **Frontend**: `hex-nexus/assets/src/components/widgets/BurnRateWidget.tsx`
- **SQL**: `SELECT SUM(cost_usd) FROM inference_log WHERE ts > ? GROUP BY …` (requires `cost_usd` column per cost-and-token-efficiency.md)

---

## (c) Cache Hit Rate Indicator

### What It Shows

**Anthropic prompt cache hit percentage** over the last 24 hours:
- **Hit rate**: `(cache_read_tokens / total_input_tokens) × 100`
- **Savings**: `(cache_read_tokens × cache_discount_rate) - (total_input_tokens × standard_rate)`

Anthropic's cache pricing (as of Jan 2025): cached input = $0.30/Mtok vs. standard input = $3.00/Mtok → **90% cost reduction** on cache hits.

### Placement

**Location**: Below Burn-Rate Widget on Mission Control (same 4-column right panel).

### Indicator Mockup

```
╔══════════════════════════════════════════╗
║  Prompt Cache Efficiency (24h)           ║
╠══════════════════════════════════════════╣
║                                          ║
║  Cache Hit Rate:   68.2%  🟢            ║
║  Tokens Cached:    1.4M                  ║
║  Cost Saved:       $3.78                 ║
║                                          ║
║  ┌──────────────────────────────────┐   ║
║  │ █████████████████░░░░░░░░░░░░░░░ │   ║ (visual bar: hit rate)
║  └──────────────────────────────────┘   ║
║  0%                              100%    ║
║                                          ║
╚══════════════════════════════════════════╝
```

### Color Logic

- **Green** 🟢: >50% hit rate
- **Yellow** 🟡: 20–50% hit rate
- **Red** 🔴: <20% hit rate

### Data Source

STDB `inference_log` needs two new columns (per cost-and-token-efficiency.md):
- `cache_read_tokens INT` (tokens served from Anthropic cache)
- `input_tokens_billed INT` (total input tokens, fresh + cached)

Query:
```sql
SELECT 
  SUM(cache_read_tokens) AS cached,
  SUM(input_tokens_billed) AS total
FROM inference_log
WHERE ts > NOW() - 86400
```

Hit rate = `(cached / total) × 100`

### Implementation Files

- **Endpoint**: `hex-nexus/src/routes/mission_control.rs::get_cache_stats()` → returns `{ hit_rate_pct, tokens_cached, cost_saved_usd }`
- **Frontend**: `hex-nexus/assets/src/components/widgets/CacheHitIndicator.tsx`
- **Backend**: `hex-nexus/src/orchestration/sop_executor.rs` must parse Anthropic API response headers `anthropic-cache-read-input-tokens` and log to STDB.

---

## Success Criteria

1. **Pre-flight modal** surfaces BEFORE high-cost SOP runs consume tokens; operator can cancel or downgrade 100% of >$0.50 runs.
2. **Burn-rate widget** updates every 5s; operator can answer "Am I over budget today?" in <3 seconds.
3. **Cache hit rate >50%** within 7 days of enabling Anthropic prompt caching (per cost-and-token-efficiency.md recommendations).
4. **Zero surprise bills**: no SOP run exceeds operator's daily `HEX_DAILY_COST_LIMIT_USD` without an approve/reject decision.

---

## Observable Artifacts

- **Frontend**:
  - `hex-nexus/assets/src/components/modals/CostPreviewModal.tsx`
  - `hex-nexus/assets/src/components/widgets/BurnRateWidget.tsx`
  - `hex-nexus/assets/src/components/widgets/CacheHitIndicator.tsx`

- **Backend**:
  - `hex-nexus/src/orchestration/sop_executor.rs::estimate_sop_cost()`
  - `hex-nexus/src/routes/mission_control.rs::get_burn_rate()` + `::get_cache_stats()`

- **STDB schema additions**:
  - `inference_log`: add `cache_read_tokens`, `input_tokens_billed`, `cost_usd` columns
  - New table: `sop_cost_override(thread_id, approved_by, approved_cost_usd, override_model, ts)`

- **Config**:
  - `HEX_COST_PREVIEW_THRESHOLD_USD` (default `0.50`)
  - `HEX_DAILY_COST_LIMIT_USD` (default `50.00`)
  - `HEX_MODEL_RATES_JSON` or hardcoded in `quant_router.rs`

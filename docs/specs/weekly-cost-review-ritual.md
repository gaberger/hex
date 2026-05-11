# Weekly Cost Review Ritual

*status*: proposed  ·  *date*: 2026-05-11

Weekly Cost Review Ritual

**Owner**: COO  
**Status**: proposed  
**Date**: 2026-05-11  
**Cadence**: Every Monday 09:00 UTC (automated + operator review by EOD)

---

## Purpose

This spec defines the **weekly cost review ritual** — a deterministic 20-minute operational audit that synthesizes token spend, persona efficiency, and cost anomaly trends from the prior 7 days. Grounds the operator in actual burn patterns and surfaces tier-policy adjustments before cost drift compounds.

References `docs/specs/cost-ops-runbook.md` §5 (weekly cost report format) and `docs/specs/coo-observability-baseline.md` §3 (cost burn vs moving average).

---

## Ritual structure

### Phase 1: Automated report generation (00:00–09:00 UTC Monday)

**Trigger**: Cron job on hex-nexus host, runs `hex cost weekly --format markdown > ~/.hex/cost-reports/YYYY-WW.md`

**Data source**: STDB `inference_log` table, window = prior Monday 00:00 UTC → Sunday 23:59 UTC

**Report sections**:
1. **Persona spend table** (7 rows: cto, cpo, ciso, coo, drafter, twin, action_executor):
   ```
   | Persona  | Runs | Total $ | Avg $/run | Top tool (by $) | Cache hit % |
   |----------|------|---------|-----------|-----------------|-------------|
   | cto      | 87   | $31.45  | $0.36     | repo_grep       | 62%         |
   | ...      | ...  | ...     | ...       | ...             | ...         |
   | **Total**| 623  | $69.95  |           |                 | **67%**     |
   ```
2. **Week-over-week delta**:
   - Compare prior week total vs 2 weeks ago: `[Δ$: +$12.30 (+21.4%)]`
   - Per-persona delta: `cto: +28% (Sonnet escalations increased)`
3. **Anomaly alerts** (auto-flagged if any row meets threshold):
   - Persona avg $/run >2× its 4-week moving average
   - Persona cache hit rate <45% (below efficiency target per `cost-and-token-efficiency.md`)
   - Total weekly spend >110% of `HEX_WEEKLY_COST_TARGET_USD` (default $350)
4. **Tool cost breakdown** (top 5 tools by cumulative spend):
   ```
   | Tool         | Calls | Total $ | Avg $/call | Personas using |
   |--------------|-------|---------|------------|----------------|
   | repo_grep    | 1243  | $18.40  | $0.015     | cto, cpo, ciso |
   | web_search   | 89    | $12.60  | $0.142     | ciso, cpo      |
   | cargo_check  | 142   | $8.20   | $0.058     | cto, drafter   |
   | ...          | ...   | ...     | ...        | ...            |
   ```
5. **Action items** (auto-generated suggestions):
   - [ ] Review CTO tier policy (Sonnet escalations +28% → consider Haiku default with complexity escalator)
   - [ ] CPO: optimize CISO SOP to batch web searches (14 calls × $0.14 = $1.96; could be 2 calls)
   - [ ] Investigate drafter cache hit rate (71% → target 80%+; check prefetch duplication)

**Output file**: `~/.hex/cost-reports/YYYY-WW.md` (e.g., `2026-W23.md`)

**Distribution**: Report posted to operator inbox at 09:00 UTC Monday with priority=med notification

---

### Phase 2: Operator review (09:00–17:00 UTC Monday)

**Operator actions** (20-min review window):
1. **Scan anomaly alerts** (top of report):
   - Red flag: any persona >2× avg → drill into `inference_log` via Mission Control `/admin/cost-metrics` persona detail view
   - Amber flag: total weekly spend >110% target → check if driven by incident response (acceptable) or SOP inefficiency (requires CPO/CTO escalation)
2. **Approve or defer action items**:
   - Operator clicks **Approve** on action item → COO persona auto-drafts mitigation (e.g., tier policy patch, SOP tweak ADR)
   - Operator clicks **Defer** → item moves to Friday backlog for next week's review
3. **Manual spot-check** (5 min):
   - Sort persona table by `Avg $/run` descending → identify outlier runs (e.g., single $4.80 run when avg is $0.36)
   - Click through to thread ID → inspect phase trace for tool loops or context bloat
   - If systemic: escalate to COO via inbox reply: *"COO: draft ADR for drafter prefetch budget cap"*
4. **Tier policy adjustment** (if needed):
   - Operator edits `~/.hex/cost-policy.yml` to pin high-burn persona to lower tier OR upgrade low-quality persona to higher tier
   - No restart required (hot-reload per `cost-ops-runbook.md` §4)
   - Change logged to `~/.hex/cost-policy-changelog.md` with rationale + timestamp

**Success criteria**:
- Operator marks review complete by clicking **Acknowledge** button in Mission Control inbox (sets `cost_review_ack_timestamp` in STDB)
- All red-flag anomalies triaged (either action item approved OR explicit defer with justification)

---

### Phase 3: COO follow-up (async, within 48 hours)

**Trigger**: Operator approves action item OR red-flag anomaly escalated

**COO persona actions**:
1. **Tier policy patch** (if operator approved action item "Review X tier policy"):
   - COO drafts `code_patch` to `~/.hex/cost-policy.yml` with new tier pin OR complexity escalator rule
   - Example: `cto: { default_tier: haiku, escalate_to_sonnet_if: "task.estimated_tokens > 12000" }`
   - Twin reviews → operator approves → executor materializes
2. **SOP efficiency ADR** (if anomaly = tool loop or context bloat):
   - COO drafts ADR with title `"<Persona> SOP contract tightening: tool call budget + prefetch cap"`
   - Decision: add `max_tool_calls_per_phase: 6` and `max_prefetch_kb: 64` to persona SOP contract
   - Consequences: reduces tail-cost risk; may require persona to escalate complex tasks to operator
3. **Post to operator digest** (Friday 17:00 UTC):
   - COO appends one-liner to weekly digest email: *"Cost review: CTO tier downgraded to Haiku (saves ~$12/week); CISO web_search batching ADR drafted (ADR-270511XXYY)"*

**Artifacts**:
- Updated `~/.hex/cost-policy.yml` (if tier adjustment)
- New ADR in `docs/adrs/` (if systemic SOP fix required)
- Action item status updated in STDB `cost_review_action_log` table

---

## Data contract

**STDB tables required**:
- `inference_log(id, role, tool_name, input_tokens, output_tokens, cost_usd, cache_hit, timestamp_utc, thread_id)` — per-call telemetry
- `cost_review_action_log(week_id, action_item, status, approver, approved_at, artifact_path)` — tracks operator decisions

**CLI command**:
```bash
hex cost weekly --format markdown --output ~/.hex/cost-reports/$(date +%Y-W%V).md
```

**Expected output schema** (Markdown):
- H1: `# Hex cost report: week NN (MMM D–D, YYYY)`
- H2: `## Persona spend` + table (7 data rows + 1 total row)
- H2: `## Week-over-week delta` + bulleted list
- H2: `## Anomaly alerts` + bulleted list (empty if none)
- H2: `## Tool cost breakdown` + table (top 5 tools)
- H2: `## Action items` + checkbox list (empty if none)

**Automation dependencies**:
- Cron job on hex-nexus host: `0 9 * * 1 cd ~/.hex && hex cost weekly --format markdown > cost-reports/$(date +%Y-W%V).md && hex inbox post --priority med --title "Weekly cost review ready" --body "file:cost-reports/$(date +%Y-W%V).md"`
- Mission Control inbox integration: displays report inline with **Acknowledge** + **Approve action item** + **Defer** buttons

---

## Alert thresholds (from `cost-ops-runbook.md` + `coo-observability-baseline.md`)

| Condition | Severity | Escalation path |
|-----------|----------|-----------------|
| Total weekly spend >110% of `HEX_WEEKLY_COST_TARGET_USD` | Amber | Operator reviews; acceptable if incident-driven, else COO tier audit |
| Total weekly spend >150% of target | Red | Operator + COO joint review; kill switch OR emergency tier downgrade |
| Any persona avg $/run >2× its 4-week MA | Red | COO drills into thread traces; drafts SOP tightening ADR if systemic |
| Any persona cache hit rate <45% | Amber | CPO investigates prefetch patterns; may need SOP refactor |
| Tool cost >$10/week AND avg $/call >$0.10 | Amber | COO reviews tool usage; flags for CTO optimization (e.g., web_search rate limit) |

---

## Success metrics

**Ritual adoption**:
- 90% of weeks have operator acknowledgment by Monday EOD (within 8 hours of report generation)
- 80% of red-flag anomalies triaged within 48 hours (action item approved OR ADR drafted)

**Cost outcomes** (measured quarterly):
- Weekly spend variance (stddev) decreases by 20% vs prior quarter (tighter tier policy → predictable burn)
- Persona avg $/run converges toward target range (e.g., CTO $0.25–$0.40, CPO $0.15–$0.30) within 6 weeks of ritual start

**Operational quality**:
- Zero surprise monthly bills (operator never sees >120% of projected monthly spend based on 4-week MA)
- Cost anomaly ADR count <2/quarter (most issues caught + mitigated in weekly review before systemic)

---

## Example walkthrough (week 23, Jun 2–8, 2025)

**Monday 09:00 UTC**: Operator opens inbox, sees:
> **Weekly cost review ready** (priority: med)  
> Total: $72.40 (+$8.10 vs week 22)  
> 🔴 CTO avg $/run = $0.68 (2.1× 4-week MA of $0.32) — **action required**  
> 🟡 Total spend = $72.40 (103% of $70 target) — **review recommended**

**Monday 10:15 UTC**: Operator clicks through to CTO detail view, sees:
- 12 Sonnet escalations (vs 3 prior week)
- Top thread: `ADR-drafting for distributed tracing` — 8 rounds, 42K input tokens, $2.80
- Root cause: drafter prefetched 6× large files (ADR-025, ADR-024, etc.) + CTO did exhaustive `repo_grep` across all crates

**Monday 10:30 UTC**: Operator approves action item:
> - [x] **COO: draft ADR for drafter prefetch budget cap (max 3 files OR 64KB per ground pack)**

**Monday 14:00 UTC**: COO persona receives approval, drafts ADR-270602XXYY with decision:
> Drafter SOP contract amendment: `max_prefetch_files: 3`, `max_prefetch_kb: 64`. On overflow, drafter must emit `repo_read` tool calls in REASON phase instead of bloating ground pack.

**Tuesday 11:00 UTC**: Twin reviews ADR → approves → executor writes to `docs/adrs/ADR-270602XXYY-drafter-prefetch-budget.md`

**Friday 17:00 UTC**: COO appends to weekly digest:
> Cost review follow-up: Drafter prefetch cap ADR shipped (ADR-270602XXYY). CTO avg $/run projected to drop to $0.42 next week. No tier policy changes needed.

**Result**: Anomaly caught early, root cause fixed within 48 hours, cost drift contained.

---

## References

- **Cost operations**: `docs/specs/cost-ops-runbook.md` (COO, 2026-05-09) — §5 weekly cost report format, §4 tier policy rotation
- **Observability baseline**: `docs/specs/coo-observability-baseline.md` (COO, 2026-05-10) — §3 cost burn vs moving average query
- **Cost surfaces**: `docs/specs/cost-and-token-efficiency.md` (CPO, 2026-05-08) — cache hit rate target (>50%), tier routing policy
- **Tool implementation**: `hex-nexus/src/tools/cost_meter.rs` — STDB `inference_log` query logic

---

## Revision history

- **2026-05-11**: Initial draft (COO) — weekly cost review ritual per operator overnight-cycle-2 directive

# COO Observability Baseline

*status*: proposed  ·  *date*: 2026-05-11

COO Observability Baseline

**Owner**: COO  
**Status**: proposed  
**Date**: 2026-05-10

---

## Purpose

This spec defines **deterministic daily-audit queries** for the COO persona to monitor hex operational health. Each metric includes: query, data source (STDB table), alert threshold, and escalation path. Grounded in `docs/specs/cost-ops-runbook.md` and `docs/specs/standup-coo-0510.md`.

---

## 1. Persona SOP failure rate by role

**Metric**: Percentage of SOP runs that fail (error / total) per persona, last 24 hours.

**Data source**: STDB `inference_log` table + `proposed_action` table

**Query**:
```sql
-- Failure rate: SOP runs where inference_log.error IS NOT NULL
SELECT 
  role AS persona,
  COUNT(*) AS total_runs,
  SUM(CASE WHEN error IS NOT NULL THEN 1 ELSE 0 END) AS failed_runs,
  ROUND(100.0 * SUM(CASE WHEN error IS NOT NULL THEN 1 ELSE 0 END) / COUNT(*), 2) AS failure_rate_pct
FROM inference_log
WHERE timestamp_utc >= NOW() - INTERVAL '24 hours'
  AND role IN ('cto', 'cpo', 'ciso', 'coo', 'drafter', 'twin', 'action_executor')
GROUP BY role
ORDER BY failure_rate_pct DESC;
```

**Alert threshold**:
- **Warn** (amber): any persona >10% failure rate
- **Escalate** (red): any persona >25% failure rate OR >5 consecutive failures

**Escalation path**:
1. Amber → COO reviews `inference_log.error` column for pattern (tool breakage, LLM refusal, timeout)
2. Red → COO escalates to operator via `escalate_to_operator(urgency=high)` with sample error messages
3. If tool breakage (e.g., `cargo_check` ENOENT from standup-coo-0510.md): escalate to CTO
4. If LLM refusal (e.g., safety filter): escalate to CPO for SOP contract review

**Automation hook**: Mission Control daily digest (09:00 UTC email) includes this table; amber/red rows highlighted

---

## 2. Workplan reconciliation drift

**Metric**: Count of ADRs with `Status: **Accepted**` that have NO corresponding workplan, or workplans marked complete whose ADR is still `Proposed`.

**Data source**: File system (`docs/adrs/*.md`, `docs/workplans/wp-*.json`) + `hex plan reconcile` command output

**Query**:
```bash
# Run via hex CLI; parses ADR Status headers + workplan phase completion
hex plan reconcile --format json > /tmp/reconcile_$(date +%F).json

# Extract drift count
jq '.drift_count' /tmp/reconcile_$(date +%F).json
```

**Manual SQL fallback** (if `hex plan reconcile` unavailable):
```sql
-- ADRs marked Accepted without workplan evidence
-- (requires ADR metadata table; not yet in STDB per ADR-[PHONE])
-- Placeholder: operator manually scans docs/adrs/ for Status: **Accepted**
-- and cross-references docs/workplans/ for matching ADR id in `adr` field
```

**Alert threshold**:
- **Warn**: drift_count >3 (≥3 ADRs out of sync with workplan state)
- **Escalate**: drift_count >10 OR any ADR accepted >14 days without workplan

**Escalation path**:
1. Warn → COO reviews `hex plan reconcile` output, identifies missing workplans
2. COO drafts missing workplans via `workplan_emit` OR escalates to drafter if ADR too complex
3. Escalate → operator intervention required; possible process breakdown (e.g., workplan_emit tool broken, twin approval stalled)

**Automation hook**: Weekly Friday 17:00 UTC report; included in operator weekly digest

---

## 3. Cost burn vs 7-day moving average

**Metric**: Today's cumulative cost vs 7-day moving average; anomaly = today >150% of MA.

**Data source**: STDB `inference_log` table, `cost_usd` column (per `docs/specs/cost-ops-runbook.md` §5)

**Query**:
```sql
WITH daily_burn AS (
  SELECT 
    DATE(timestamp_utc) AS date,
    SUM(cost_usd) AS total_cost_usd
  FROM inference_log
  WHERE timestamp_utc >= NOW() - INTERVAL '8 days'
  GROUP BY DATE(timestamp_utc)
),
ma_7day AS (
  SELECT AVG(total_cost_usd) AS ma_7day_usd
  FROM daily_burn
  WHERE date < CURRENT_DATE
)
SELECT 
  d.total_cost_usd AS today_usd,
  m.ma_7day_usd,
  ROUND(100.0 * d.total_cost_usd / m.ma_7day_usd, 2) AS today_vs_ma_pct
FROM daily_burn d
CROSS JOIN ma_7day m
WHERE d.date = CURRENT_DATE;
```

**Alert threshold** (per `cost-ops-runbook.md` §3):
- **Warn**: today_vs_ma_pct >150% (1.5× moving average)
- **Escalate**: today_vs_ma_pct >300% (3× moving average) — cost anomaly triage required

**Escalation path**:
1. Warn → COO reviews persona breakdown (query §1 above, group by persona)
2. Identifies high-burn persona; checks for tool loops (`repo_grep` >6 calls per SOP run)
3. Escalate → follows `cost-ops-runbook.md` §3 decision tree: kill switch OR tier downgrade OR post-incident ADR

**Automation hook**: Mission Control dashboard live tile; updates every 5 minutes during business hours (08:00–18:00 UTC)

---

## 4. STDB reducer tick anomalies

**Metric**: Last successful tick timestamp for critical reducers; alert if any reducer silent >10 minutes.

**Data source**: STDB system tables (e.g., `__reducers__` or custom `reducer_heartbeat` table; assumes ADR-[PHONE] schema)

**Query**:
```sql
-- Placeholder: actual STDB schema TBD by CTO
-- Assumes reducer_heartbeat(reducer_name, last_tick_utc) table exists
SELECT 
  reducer_name,
  last_tick_utc,
  EXTRACT(EPOCH FROM (NOW() - last_tick_utc)) / 60 AS minutes_since_last_tick
FROM reducer_heartbeat
WHERE reducer_name IN (
  'commitment_reverify_tick',
  'twin_reviewer',
  'action_executor',
  'sop_executor',
  'cost_aggregator'
)
ORDER BY minutes_since_last_tick DESC;
```

**Alert threshold**:
- **Warn**: any reducer >10 min since last tick
- **Escalate**: any reducer >30 min OR `action_executor` >5 min (blocking materialization per ADR-[PHONE])

**Escalation path**:
1. Warn → COO checks STDB logs for reducer crash / panic stack trace
2. Escalate → COO invokes kill switch: `export HEX_DISABLE_<REDUCER>=true` + restart hex-nexus
3. Operator notified via `escalate_to_operator(urgency=high, reason="action_executor stalled; proposed_actions not materializing")`

**Automation hook**: Mission Control `/admin/reducers` page; red banner if any reducer >10 min silent

---

## 5. Digital-twin rejection rate

**Metric**: Percentage of `proposed_action` rows rejected by digital twin (approved=false) in last 24 hours.

**Data source**: STDB `proposed_action` table, `approved` column + `twin_review_timestamp` (schema per ADR-[PHONE])

**Query**:
```sql
SELECT 
  COUNT(*) AS total_proposals,
  SUM(CASE WHEN approved = FALSE THEN 1 ELSE 0 END) AS rejected,
  ROUND(100.0 * SUM(CASE WHEN approved = FALSE THEN 1 ELSE 0 END) / COUNT(*), 2) AS rejection_rate_pct
FROM proposed_action
WHERE twin_review_timestamp >= NOW() - INTERVAL '24 hours';
```

**Alert threshold** (per `cost-ops-runbook.md` §4 quality drop):
- **Warn**: rejection_rate_pct >15%
- **Escalate**: rejection_rate_pct >25% OR >10 consecutive rejections

**Escalation path**:
1. Warn → COO reviews `proposed_action.rejection_reason` column (sample 5 recent rejections)
2. Common patterns:
   - **Hallucinated file paths** → drafter SOP needs tighter grounding contract (CPO domain)
   - **Policy violation** (e.g., writes to excluded dir) → twin config issue (CTO domain)
   - **Malformed JSON** → LLM tier too low (Haiku struggles with complex schema) → upgrade tier per `cost-policy.yml`
3. Escalate → COO drafts ADR if systemic (e.g., SOP contract flaw) OR escalates to CPO/CTO per pattern

**Automation hook**: Weekly Friday report; included in operator digest with top-3 rejection reasons

---

## 6. Tool health matrix

**Metric**: Success rate per tool (% of calls that returned ok=true) in last 24 hours.

**Data source**: STDB `tool_invocation_log` table (schema TBD; assumes each tool call logged with `tool_name`, `ok`, `error`, `timestamp_utc`)

**Query**:
```sql
SELECT 
  tool_name,
  COUNT(*) AS total_calls,
  SUM(CASE WHEN ok = TRUE THEN 1 ELSE 0 END) AS successful_calls,
  ROUND(100.0 * SUM(CASE WHEN ok = TRUE THEN 1 ELSE 0 END) / COUNT(*), 2) AS success_rate_pct
FROM tool_invocation_log
WHERE timestamp_utc >= NOW() - INTERVAL '24 hours'
GROUP BY tool_name
ORDER BY success_rate_pct ASC, total_calls DESC;
```

**Alert threshold**:
- **Warn**: any tool with >20 calls AND success_rate_pct <80%
- **Escalate**: any tool with success_rate_pct <50% OR critical tool (cargo_check, repo_read, repo_grep) <90%

**Escalation path** (per `standup-coo-0510.md` lesson:tooling-resilience):
1. Warn → COO reviews `tool_invocation_log.error` column for pattern (e.g., `cargo_check` ENOENT, `web_search` API key missing)
2. Non-critical tool (<80%) → COO files issue for CTO; persona SOP may need fallback logic
3. Critical tool (<90%) → COO escalates to operator immediately: `escalate_to_operator(urgency=high, reason="cargo_check tool broken; code quality gate down")`
4. Operator invokes Tool Czar persona (ADR-[PHONE]) for automated health probe + mitigation

**Automation hook**: Mission Control `/admin/tools` page; table sorted by success_rate_pct ASC; red rows for <80%

---

## 7. Operational cadence summary

| Metric | Frequency | Data source | Alert threshold | Escalation target |
|--------|-----------|-------------|-----------------|-------------------|
| **Persona SOP failure rate** | Daily 09:00 UTC | `inference_log.error` | >10% warn, >25% escalate | Operator (high urgency) |
| **Workplan reconciliation drift** | Weekly Friday 17:00 UTC | `hex plan reconcile` CLI | >3 warn, >10 escalate | COO (draft workplan) → Operator |
| **Cost burn vs MA** | Live (5-min refresh) | `inference_log.cost_usd` | >150% warn, >300% escalate | COO (tier downgrade) → Operator |
| **STDB reducer ticks** | Live (1-min refresh) | `reducer_heartbeat` | >10 min warn, >30 min escalate | Operator (reducer restart) |
| **Digital-twin rejection rate** | Daily 09:00 UTC | `proposed_action.approved` | >15% warn, >25% escalate | CPO (SOP fix) OR CTO (twin config) |
| **Tool health matrix** | Daily 09:00 UTC | `tool_invocation_log` | <80% warn, <90% critical escalate | CTO (tool fix) OR Tool Czar |

---

## 8. Mission Control dashboard integration

**Proposed UX** (CPO domain; COO spec defines data contract):

- **`/admin/observability`** page with six live tiles (one per metric above)
- Each tile shows:
  - Current value (numeric + trend arrow ↑↓)
  - Alert status (green / amber / red)
  - "Drill down" button → opens detailed query result table
- **Inbox integration**: amber/red alerts auto-generate operator inbox notification with priority=2 (med) or priority=1 (high)
- **Weekly digest email**: Markdown table aggregating all six metrics; sent Monday 09:00 UTC

---

## 9. References

- **Cost operations**: `docs/specs/cost-ops-runbook.md` (COO, 2026-05-09) — cost anomaly triage, escalation matrix
- **Standup lesson**: `docs/specs/standup-coo-0510.md` (COO, 2026-05-10) — tooling resilience, cargo_check gate failure
- **Digital twin**: ADR-[PHONE] (CTO) — proposed_action approval workflow
- **Tool library**: ADR-[PHONE] (CTO) — SOP contract, tool registration
- **Workplan reconciliation**: ADR-[PHONE] (CTO) — `hex plan reconcile` command spec
- **Tool Czar**: ADR-[PHONE] (CTO) — automated tool health monitoring persona

---

## 10. Implementation notes

**STDB schema gaps** (CTO domain):
- `inference_log.cost_usd` column — specified in `cost-ops-runbook.md` §5, not yet materialized
- `proposed_action.approved`, `twin_review_timestamp`, `rejection_reason` columns — ADR-[PHONE] schema TBD
- `reducer_heartbeat` table OR `__reducers__.last_tick_utc` column — STDB system introspection surface
- `tool_invocation_log` table — each tool call logged with `ok`, `error`, `timestamp_utc`, `tool_name`

**CLI dependencies**:
- `hex plan reconcile --format json` — ADR-[PHONE] workplan tooling (CTO domain)
- `hex cost weekly --persona all --breakdown` — `cost-ops-runbook.md` §5 (CPO domain)

**Automation dependencies**:
- Mission Control `/admin/observability` page — CPO domain (UX spec)
- Weekly email digest generator — CPO domain (cron job + email template)
- Inbox notification triggers — CPO domain (STDB reducer watches alert thresholds)

---

## Revision history

- **2026-05-10**: Initial draft (COO) — deterministic daily-audit queries per operator directive
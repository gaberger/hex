# Morning Health Check Operational Ritual

*status*: proposed  ·  *date*: 2026-05-11

Morning Health Check Operational Ritual

**Owner**: COO  
**Status**: proposed  
**Date**: 2026-05-11  
**Frequency**: Daily, 09:00 UTC (automated report + operator review)

---

## Purpose

This spec defines the **morning health check** operational ritual: an automated 5-minute deterministic audit that surfaces overnight anomalies, SOP failures, cost overruns, and infrastructure drift. The operator reviews a single consolidated report and escalates exceptions. Grounded in `docs/specs/cost-ops-runbook.md` and `docs/specs/coo-observability-baseline.md`.

---

## Ritual Structure

### Phase 1: Automated Data Collection (00:00–09:00 UTC)

Background STDB reducers aggregate overnight metrics. No operator action required.

**Data collected**:
1. **SOP run summary** (`inference_log` table): total runs, failures, cost per persona
2. **Tool health** (`tool_invocation_log` table): success rates for critical tools (cargo_check, repo_read, repo_grep, cargo_audit, web_search)
3. **Digital-twin review** (`proposed_action` table): approval rate, rejection reasons
4. **Cost burn** (`inference_log.cost_usd`): 24-hour total vs 7-day moving average
5. **STDB reducer ticks** (`reducer_heartbeat` table): last-tick timestamp for critical reducers
6. **Workplan drift** (`hex plan reconcile` output): ADRs accepted without workplan evidence

### Phase 2: Report Generation (09:00 UTC)

STDB reducer `morning_health_check_report` runs at 09:00 UTC, executes six queries from `coo-observability-baseline.md` §1–§6, and writes output to:
- **STDB**: `health_check_report(date, status, summary_json)` table
- **Email**: Markdown table sent to operator inbox (if configured)
- **Mission Control**: `/admin/health` page auto-refreshes with latest report

### Phase 3: Operator Review (09:00–09:15 UTC)

Operator opens Mission Control `/admin/health` page or email digest. Review takes 5 minutes:

**Green status** (all thresholds met):
```
✓ All systems nominal
✓ 0 amber alerts, 0 red alerts
✓ Overnight cost: $12.40 (78% of MA)
✓ SOP success rate: 96.2%
✓ Tool health: 98.7% avg success rate
✓ Digital-twin approval rate: 91%
```
**Action**: No operator action required. Close report.

**Amber status** (1+ warn thresholds crossed):
```
⚠ 2 amber alerts detected

[A1] SOP failure rate: drafter 12.5% (threshold: 10%)
     Last 3 errors: "tool call timeout: repo_grep exceeded 60s"
     → COO action: Review drafter SOP for grep pattern complexity

[A2] Cost burn: $41.20 (164% of MA $25.10)
     High-burn persona: cto ($28.30, 68% of daily total)
     → COO action: Review CTO tier policy; consider Haiku default
```
**Action**: Operator reviews COO-proposed mitigation (inline in report). Approves OR escalates to relevant persona (CTO/CPO/CISO).

**Red status** (1+ escalate thresholds crossed):
```
🚨 1 critical alert detected

[R1] Tool health: cargo_check 47% success rate (threshold: 90%)
     Error pattern: "ENOENT: /usr/bin/cargo not found" (8 occurrences)
     → ESCALATE TO OPERATOR: Infrastructure breakage; code quality gate down
```
**Action**: Operator clicks **Escalate** button → opens incident form → assigns to on-call CTO OR invokes Tool Czar persona (ADR-[PHONE]) for automated mitigation.

---

## Report Schema

**Markdown template** (email + Mission Control page):

```markdown
# Hex Morning Health Check — YYYY-MM-DD

**Status**: 🟢 Green / ⚠ Amber / 🚨 Red  
**Generated**: 09:00 UTC  
**Overnight window**: [yesterday 09:00] → [today 09:00]

---

## Summary

| Metric | Value | Threshold | Status |
|--------|-------|-----------|--------|
| **SOP runs** | 142 total, 4 failed | <10% failure rate | 🟢 2.8% |
| **Cost burn** | $12.40 | <150% of MA ($15.80) | 🟢 78% |
| **Tool health** | 98.7% avg success | >90% critical tools | 🟢 Pass |
| **Twin approval rate** | 91% | >85% | 🟢 Pass |
| **Reducer ticks** | All <5 min ago | <10 min | 🟢 Pass |
| **Workplan drift** | 2 ADRs out of sync | <3 | 🟢 Pass |

---

## Alerts

*None* — all metrics within operational thresholds.

---

## Top overnight activity

| Persona | Runs | Cost $ | Avg $/run | Top tool |
|---------|------|--------|-----------|----------|
| cto     | 38   | $5.20  | $0.14     | repo_grep |
| drafter | 54   | $3.10  | $0.06     | repo_read |
| twin    | 142  | $1.80  | $0.01     | (internal) |
| ciso    | 4    | $1.40  | $0.35     | cargo_audit |
| cpo     | 6    | $0.90  | $0.15     | repo_read |

---

## Recommended actions

*None* — operator may proceed with normal tasking.

---

**Drill-down links**:
- [Persona failure details](/admin/observability?metric=sop_failures)
- [Cost breakdown by tool](/admin/observability?metric=cost_burn)
- [Tool health matrix](/admin/tools)
- [Workplan reconciliation](/admin/workplans/reconcile)
```

---

## Escalation Decision Tree

**Green** → No action; archive report  
**Amber** → COO reviews inline mitigation → Operator approves OR reassigns to domain owner  
**Red** → Operator creates incident ticket → Assigns to on-call persona OR invokes Tool Czar

### Amber → Domain Owner Mapping

| Alert category | Domain owner | Escalation path |
|----------------|--------------|-----------------|
| SOP failure >10% | CPO (SOP contract) OR CTO (tool breakage) | COO reviews `inference_log.error` → escalates with sample errors |
| Cost burn >150% MA | COO (tier policy) | COO drafts tier downgrade OR escalates to operator if anomaly unexplained |
| Tool health 80–90% | CTO (tool implementation) | COO files issue with error pattern; CTO triages within 24 hours |
| Twin rejection >15% | CPO (SOP grounding) OR CTO (twin policy) | COO reviews `rejection_reason` → escalates with top-3 patterns |
| Workplan drift >3 | COO (missing workplans) | COO drafts workplans via `workplan_emit` OR escalates to drafter if complex |

### Red → Incident Response

| Alert category | Urgency | Incident owner | SLA |
|----------------|---------|----------------|-----|
| Tool health <50% | **High** | CTO + Tool Czar | 4 hours to restore >90% |
| Reducer tick >30 min | **High** | CTO (STDB infra) | 1 hour to restore tick |
| Cost burn >300% MA | **Med** | COO + Operator | 24 hours to identify root cause |
| SOP failure >25% | **Med** | CPO (SOP) + CTO (tool) | 24 hours to restore <10% |

---

## Automation Requirements

**STDB reducer**: `morning_health_check_report`
- **Trigger**: Cron schedule, daily 09:00 UTC
- **Dependencies**: `inference_log`, `proposed_action`, `tool_invocation_log`, `reducer_heartbeat` tables
- **Output**: Writes `health_check_report(date, status, summary_json, alerts_json)` row

**Mission Control page**: `/admin/health`
- **Data source**: Latest `health_check_report` row from STDB
- **Refresh**: Auto-refresh on page load (no polling; report static until next 09:00 UTC run)
- **UX**: Markdown rendering + expandable alert sections + drill-down links to observability metrics

**Email integration** (optional):
- **Recipient**: Operator email from `~/.hex/config.toml`
- **Condition**: Send only if status=Amber OR status=Red (suppress Green emails to reduce noise)
- **Format**: Plain Markdown (same template as Mission Control page)

---

## Manual Override

Operator can trigger ad-hoc health check via CLI:

```bash
hex admin health-check --now
```

**Behavior**:
1. Runs six queries from `coo-observability-baseline.md` against current STDB state
2. Writes `health_check_report` row with `date=CURRENT_TIMESTAMP`, `is_adhoc=true`
3. Prints Markdown report to stdout
4. Does NOT email (manual invocation; operator already at terminal)

**Use case**: Post-incident validation (e.g., after CTO fixes `cargo_check` tool, operator runs `hex admin health-check --now` to confirm all metrics green).

---

## Success Criteria

1. **Operator time**: Morning review takes <5 minutes for Green status, <15 minutes for Amber (including COO mitigation review)
2. **Escalation accuracy**: Red alerts correlate with genuine incidents (false-positive rate <5% measured over 30 days)
3. **Automation coverage**: 100% of queries run deterministically; no manual SQL required
4. **Incident detection latency**: Critical issues (Red alerts) detected within 24 hours (overnight window)

---

## References

- **COO observability baseline**: `docs/specs/coo-observability-baseline.md` (six deterministic queries, alert thresholds, escalation paths)
- **Cost operations runbook**: `docs/specs/cost-ops-runbook.md` (cost anomaly triage, tier policy, weekly review cadence)
- **Tool Czar persona**: ADR-[PHONE] (automated tool health monitoring)
- **Workplan reconciliation**: ADR-[PHONE] (`hex plan reconcile` command spec)
- **Digital-twin approval**: ADR-[PHONE] (`proposed_action` table schema)

---

## Implementation Notes

**STDB schema dependencies** (CTO domain):
- `health_check_report(date, status, summary_json, alerts_json, is_adhoc)` table
- `reducer_heartbeat(reducer_name, last_tick_utc)` table (or equivalent STDB introspection API)
- `inference_log.cost_usd` column (per `cost-ops-runbook.md` §5)
- `tool_invocation_log(tool_name, ok, error, timestamp_utc)` table

**CLI command** (CTO domain):
- `hex admin health-check --now` — triggers ad-hoc report generation

**Mission Control UX** (CPO domain):
- `/admin/health` page with Markdown rendering + alert drill-down links
- Email template integration (conditional send on Amber/Red)

---

## Revision History

- **2026-05-11**: Initial draft (COO) — morning health check ritual per operator overnight cycle 3
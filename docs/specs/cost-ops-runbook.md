# Cost Ops Runbook

*status*: proposed  ·  *date*: 2026-05-09

Cost Ops Runbook

**Owner**: COO  
**Status**: proposed  
**Date**: 2025-06-05

---

## Purpose

This runbook defines **how the hex organization operates** around the cost and token efficiency surfaces specified in `docs/specs/cost-and-token-efficiency.md`. CPO defined what the operator sees; this runbook defines operational procedures, escalation paths, kill switches, and audit cadence.

---

## 1. On-call procedure: daily burn exceeds `HEX_DAILY_COST_LIMIT_USD`

**Threshold**: `HEX_DAILY_COST_LIMIT_USD` (operator-configured; default `$50/day`)

**Detection**: Mission Control dashboard `/admin/cost-metrics` displays alert banner when cumulative daily spend crosses threshold (data sourced from STDB `inference_log` aggregation).

**Auto-engagement**:
- **Cost gates activate**: All pending SOP runs with estimated cost > `HEX_SOP_COST_GATE_USD` (default `$1.00`) are auto-held and appear in operator inbox with override button.
- **No paging**: hex is an operator-supervised AIOS; burn limit breach is a **dashboard alert**, not a PagerDuty event. Operator checks Mission Control on their cadence (typically hourly during active dev).

**Operator actions**:
1. **Review cost breakdown table** (`[Persona | Today $ | MTD $ | Avg $/run]`) to identify high-burn persona.
2. **Check for anomalies**: sort by `Avg $/run`; if any persona exceeds 3× its 7-day average, proceed to **§3 Cost Anomaly Triage**.
3. **Approve or reject held SOP runs**: click override button for business-critical runs; reject exploratory/low-priority threads.
4. **Temporary tier downgrade**: edit `~/.hex/cost-policy.yml` to pin high-burn persona to `haiku` or `local` tier until daily window resets (UTC midnight).

**Mission Control UX**:
- Alert banner (amber): *"Daily cost limit exceeded: $53.20 / $50.00. 4 SOP runs held pending approval."*
- Held runs appear as inbox notifications with metadata: `[persona | estimated_cost | reason snippet]`
- Operator clicks **Approve** → sets `sop_run_override` flag in STDB → next poll proceeds with REASON phase.

---

## 2. Escalation matrix: high-cost SOP run override authority

| Cost tier | Approval authority | Max timeout | Notes |
|-----------|-------------------|-------------|-------|
| **< $1** | Auto-approved | N/A | Below gate; no operator intervention |
| **$1–$5** | **Operator** self-approves via dashboard | 4 hours | Routine overrides for deep analysis tasks |
| **$5–$20** | **COO** approval required | 24 hours | COO reviews justification in thread context; approves via `hex admin approve-cost <thread_id>` CLI command |
| **$20–$50** | **CPO** (product value) + **Operator** (business) joint approval | 48 hours | Reserved for multi-crate refactors, whole-codebase paradigm shifts |
| **> $50** | **Operator veto** | N/A | No persona may run a single SOP >$50 without explicit `HEX_OVERRIDE_MAX_COST_USD` env var change + restart |

**Override claim procedure**:
- COO claims via CLI: `hex admin approve-cost <thread_id> --approver coo --justification "incident response: CVE triage across 6 ADRs"`
- CPO claims via dashboard: clicks **Escalate to CPO** button, adds product rationale in text field
- All approvals logged to STDB `cost_override_log(thread_id, approver, justification, timestamp, approved_cost_usd)`

**Timeout behavior**:
- If approval not granted within max timeout, SOP run auto-expires
- Thread status set to `expired_cost_gate`
- Operator sees notification: *"Thread #4721 expired; resubmit with cost justification if still needed"*

---

## 3. Cost anomaly triage: single SOP run exceeds 3× persona average

**Trigger**: Any SOP run where `(actual_cost_usd > persona_7day_avg * 3)`

**Detection**: Post-run hook in `sop_executor.rs` emits `cost_anomaly_detected` event to STDB; Mission Control inbox shows priority notification.

**Triage steps**:
1. **Review phase trace**: operator clicks thread → inspects REASON round-trip count, tool call frequency, output token usage.
2. **Identify root cause**:
   - **Tool loop**: 8× `repo_grep` calls with overlapping patterns → drafter hallucinated complex search strategy
   - **Context bloat**: ground pack prefetch pulled 6× 32KB files → input token spike
   - **Output verbosity**: LLM generated 8KB response when 2KB sufficed → `max_tokens` too high
3. **Immediate mitigation** (pick one):
   - **Kill switch (persona-level)**: `export HEX_DISABLE_<PERSONA>=true` (e.g., `HEX_DISABLE_DRAFTER=true`) per ADR-2604142100 — disables that loop until operator re-enables
   - **Kill switch (twin/executor)**: `HEX_DISABLE_TWIN=true` or `HEX_DISABLE_ACTION_EXECUTOR=true` — pauses all twin reviews or all action execution
   - **Tier downgrade**: edit `cost-policy.yml` to force `local` tier for anomalous persona until root cause fixed
4. **Post-incident**: COO drafts ADR if systemic (e.g., SOP contract needs tighter `max_tokens` enforcement), or CPO updates persona SOP if behavioral (e.g., drafter over-using `repo_grep`).

**Runbook decision tree**:
```
Anomaly detected → Review trace
  ├─ Tool loop (>6 calls same tool) → Kill switch + ADR for tool rate limit
  ├─ Context bloat (>64KB input) → Reduce prefetch budget in persona SOP
  └─ Output verbosity (>4K tokens) → Lower `max_tokens` in cost-policy.yml
```

---

## 4. Tier-policy rotation cadence

**File**: `~/.hex/cost-policy.yml`  
**Owner**: Operator (with COO consultation on thresholds)

**Review cadence**:
- **Weekly**: operator reviews persona cost breakdown on Friday EOD; adjusts tier pins if any persona exceeds weekly target (e.g., CTO >$100/week → downgrade to Haiku)
- **On quality regression**: if persona produces 3+ failed `cargo_check` or malformed JSON in a week, escalate tier (e.g., `local` → `haiku` → `sonnet`)
- **On model deprecation**: when cloud provider sunsets model (e.g., Anthropic deprecates Haiku 3.5), COO updates policy to next-gen equivalent within 48 hours

**Trigger for re-pin**:
- **Cost overrun**: persona burns >150% of allocated weekly budget → downgrade tier
- **Quality drop**: persona `proposed_action` rejection rate >25% → upgrade tier
- **Model sunset**: provider announces EOL date → COO posts in operator Discord, schedules policy update sprint

**Change control**:
- Operator commits `cost-policy.yml` changes to `~/.hex/` (not version-controlled; instance-specific)
- Restart not required; `sop_executor.rs` hot-reloads policy on next SOP poll (every 20s)

---

## 5. Audit cadence: weekly cost report

**Frequency**: Every Monday 09:00 UTC (automated)

**Data source**: STDB `inference_log` table, aggregated by `persona`, `tool_name`, `date`

**Report format** (Markdown table emailed to operator):
```
# Hex cost report: week 23 (Jun 2–8, 2025)

| Persona  | Runs | Total $ | Avg $/run | Top tool (by $) | Cache hit % |
|----------|------|---------|-----------|-----------------|-------------|
| cto      | 87   | $31.45  | $0.36     | repo_grep       | 62%         |
| cpo      | 64   | $18.20  | $0.28     | repo_read       | 58%         |
| ciso     | 12   | $6.80   | $0.57     | web_search      | 41%         |
| drafter  | 142  | $9.30   | $0.07     | (internal)      | 71%         |
| twin     | 318  | $4.20   | $0.01     | (internal)      | 83%         |
| **Total**| 623  | $69.95  |           |                 | **67%**     |

**Alerts**:
- CTO avg $/run up 28% vs prior week (Sonnet escalations increased)
- CISO cache hit rate low (41%); investigate `web_search` call patterns

**Action items**:
- [ ] Review CTO tier policy (consider Haiku default with complexity escalator)
- [ ] CPO: optimize CISO SOP to batch web searches
```

**Consumer**: Operator (reviews Monday AM); COO (async, flags systemic issues for ADR)

**Storage**: Reports archived to `~/.hex/cost-reports/YYYY-WW.md` for historical trend analysis

---

## 6. Operational thresholds (concrete numbers)

| Parameter | Default | Rationale |
|-----------|---------|-----------|
| `HEX_DAILY_COST_LIMIT_USD` | `$50` | Caps monthly burn at ~$1,500 (100 work days/year × $50) |
| `HEX_SOP_COST_GATE_USD` | `$1.00` | One Sonnet SOP run at current rates; blocks runaway 8-round-trip scenarios |
| `HEX_WEEKLY_COST_TARGET_USD` | `$350` | 7 days × $50; operator reviews if actual >110% of target |
| **Paging window** | None | Hex is operator-supervised; no after-hours paging. Operator checks dashboard on their schedule. |
| **Override approval timeout** | 4h (≤$5), 24h (≤$20), 48h (≤$50) | Balances urgency (incident response) vs cost control |
| **Anomaly threshold** | 3× persona 7-day avg | Catches outliers (e.g., $1.50 run when avg is $0.40) without false positives on normal variance |
| **Cache TTL** | 5 min (file content), 1 hour (Anthropic prompt cache) | Balances token savings vs stale data risk |
| **Tier rotation review** | Weekly (Fridays) | Aligns with weekly cost report cycle |

---

## 7. References

- **Product spec**: `docs/specs/cost-and-token-efficiency.md` (CPO-owned; defines UX surfaces)
- **Kill switches**: ADR-2604142100 (persona disable flags)
- **Tier routing**: `hex-nexus/src/quant_router.rs` (CTO-owned implementation)
- **Inference logging**: STDB `inference_log` schema (needs `cost_usd` column per product spec)

---

## Revision history

- **2025-06-05**: Initial draft (COO) — operational procedures around CPO's cost surfaces

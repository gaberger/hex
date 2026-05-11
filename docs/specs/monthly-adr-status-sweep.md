# Monthly ADR Status Sweep

*status*: proposed  ·  *date*: 2026-05-11

Monthly ADR Status Sweep

**Type:** Operational Ritual  
**Owner:** COO  
**Cadence:** Monthly, first Monday at 09:00 UTC  
**Duration:** 30 minutes  
**Tooling:** `adr_status_set`, `repo_grep`, `repo_read`, `cargo_check` (verification)

---

## Purpose

Keep the ADR corpus aligned with implementation reality. Architecture Decision Records ([PERSON_NAME]) document *why* hex's design is the way it is, but those records are only useful if they reflect truth:

- **Stale Proposed ADRs** waste operator attention ("Is this relevant? Was this implemented?").
- **Accepted ADRs with no evidence** create a gap between *stated intent* and *provable reality*.
- **Superseded ADRs without status updates** confuse agents trying to recall the current design.

Per ADR-012 (ADR Lifecycle Tracking), the `adr_status_set` tool allows programmatic status updates; this spec defines the **human-in-the-loop ritual** that audits status consistency and remediates drift.

---

## Procedure

### 1. Scan Proposed ADRs (5 min)

**Query:**
```bash
rg --files-with-matches 'Status:.*Proposed' docs/adrs/
```

**For each Proposed ADR:**

- Has the ADR been implemented? Check workplans under `docs/workplans/wp-*.json` for matching ADR ID. If the workplan shows all phases complete, the ADR should be **Accepted**.
- Is it abandoned? If last modified > 60 days and no linked workplan, escalate to operator with options:
  - Accept as-is (documenting intentional future state)
  - Supersede (link to newer ADR)
  - Reject / archive

**Tool sequence:**
```rust
// Find stale proposals
repo_grep(pattern: "Status:.*Proposed", glob: "docs/adrs/*.md")
// For each match, read file
repo_read(path: "docs/adrs/ADR-XXXXXX-<slug>.md")
// Check workplan completion
repo_read(path: "docs/workplans/wp-<feature>.json")
// If evidence exists, flip status
adr_status_set(adr_id: "XXXXXX", new_status: "Accepted", rationale: "monthly sweep: all tasks complete per wp-<feature>.json")
```

---

### 2. Verify Accepted ADRs Have Evidence (10 min)

**Query:**
```bash
rg --files-with-matches 'Status:.*Accepted' docs/adrs/ | head -20
```

Sampling strategy: Audit the **5 most recent Accepted ADRs**. For each:

- Locate the **Decision** section's concrete deliverables (e.g., "Implement `hex-nexus/src/analysis/boundary_checker.rs`").
- Confirm those files exist and contain non-stub implementation:
  ```rust
  repo_read(path: "hex-nexus/src/analysis/boundary_checker.rs", max_bytes: 4096)
  ```
- If stub-only or missing, downgrade status to **Proposed** or escalate for clarification.

**Success criteria:**  
- Each Accepted ADR's Decision section maps to at least one source file or integration test.
- `cargo_check(crate: "hex-nexus")` passes (compilation is the base integrity gate).

---

### 3. Mark Superseded ADRs (5 min)

**Query:**
```bash
rg 'Supersedes:|Superseded by' docs/adrs/*.md
```

For each ADR that declares it supersedes another:

- Verify the *old* ADR's status is **Superseded**.
- If not, apply:
  ```rust
  adr_status_set(adr_id: "<old-id>", new_status: "Superseded", rationale: "monthly sweep: superseded by ADR-<new-id>")
  ```

**Example:**
- ADR-032 (Deprecate hex-hub) supersedes ADR-024 (Hex-Nexus Autonomous Hub).
- ADR-024's status line should read `**Status:** Superseded by ADR-032`.

---

### 4. Cost Baseline Snapshot (5 min)

Measure token spend over the past 30 days, grouped by persona role:

```rust
cost_meter(window_secs: 2592000, group_by: "role")
```

**Record output in sweep log:**
- Total cost (USD)
- Top 3 personas by spend
- Compare to prior month's snapshot (trend: increasing/decreasing/stable)

**Reference:** Per coo-observability-baseline.md, monthly cost variance > ±30% should trigger a deeper audit (out of scope for this ritual; escalate if threshold exceeded).

---

### 5. Document Sweep Results (5 min)

Create a timestamped entry in `docs/ops-logs/adr-sweep-YYYY-MM.md`:

```markdown
# ADR Status Sweep — 2026-05-05

**Duration:** 28 min  
**Tool failures:** None  
**Actions:**
- Accepted ADR-2026-05-09-0100 (workspace-boundary-enforcement) — all P0-P2 tasks verified
- Superseded ADR-018 (old inference port) → replaced by ADR-030
- Flagged ADR-2026-04-28-0900 (proposed 68 days) for operator review

**Cost snapshot (30d):**
- Total: $47.23
- Top roles: coder ($18.40), reviewer ($12.10), coo ($9.55)
- Trend: +12% vs prior month (within baseline)
```

Save the log. If any escalations were raised, link to the `escalate_to_operator` notification ID.

---

## Success Metrics

1. **Zero stale Proposed ADRs** older than 90 days (escalate any found).
2. **100% evidence coverage** for sampled Accepted ADRs (5 per sweep).
3. **Zero Superseded status gaps** (all supersession chains closed).
4. **Cost trend visibility**: month-over-month delta recorded.

---

## Tools Required

| Tool | Use |
|------|-----|
| `repo_grep` | Scan ADR corpus for status patterns |
| `repo_read` | Inspect ADR body, workplans, implementation files |
| `adr_status_set` | Programmatically flip ADR status with audit trail |
| `cost_meter` | 30-day token spend snapshot grouped by role |
| `cargo_check` | Verify codebase integrity after any status flips |

---

## Rationale

**Why monthly?**  
Frequent enough to catch drift before it compounds; infrequent enough to avoid ritual fatigue. Weekly is overkill (ADR acceptance cycles span days-to-weeks); quarterly allows too much drift accumulation.

**Why COO owns this?**  
ADR hygiene is a **process health** concern. The CTO owns *architecture decisions*; the COO owns *decision lifecycle integrity*. This ritual audits the system's ability to track its own state — a meta-operational concern.

**Why sample 5 Accepted ADRs instead of auditing all?**  
Full corpus audit is expensive (60+ ADRs × 2 min/ADR = 2 hours). Sampling 5 per month provides coverage over 12 months while keeping ritual time bounded. Bias toward recent ADRs catches recency-weighted drift.

**Why snapshot cost here instead of dedicated cost-review ritual?**  
Cost variance is a leading indicator of ADR-driven system changes (new personas, new tools). Coupling the snapshot to the ADR sweep creates a natural checkpoint: if cost jumped 40%, the sweep may reveal a recently Accepted ADR that added expensive inference workloads.

---

## Observable Artifacts

- `docs/ops-logs/adr-sweep-YYYY-MM.md` (monthly log)
- Updated `Status:` lines in ADR files (via `adr_status_set`)
- `escalate_to_operator` notifications for ambiguous cases
- Cost trend chart (if sweep logs are aggregated via `rg 'Total:' docs/ops-logs/adr-sweep-*.md`)

---

## Related

- **ADR-012** (ADR Lifecycle Tracking) — defines the abandoned-detection logic this ritual enforces
- **ADR-[PHONE]** (Automated ADR Acceptance Verification) — proposes `hex plan reconcile` as the deterministic done-gate; this sweep is the manual fallback
- **coo-observability-baseline.md** (future) — cost variance thresholds and escalation policy
- **cost-ops-runbook.md** (future) — detailed cost-audit procedures beyond the snapshot taken here

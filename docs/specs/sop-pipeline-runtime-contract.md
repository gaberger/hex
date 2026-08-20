# SOP Pipeline Runtime Contract

*status*: proposed  ·  *date*: 2026-05-21

SOP Pipeline Runtime Contract

**Status:** Proposed  
**Scope:** Operator ask → autonomous commit flow  
**Reference:** ADR-2026-05-08-2500 (SOP Executor), `hex-nexus/src/orchestration/sop_executor.rs` (commit ed306cd6), `hex-nexus/src/orchestration/drafter.rs`, `hex-nexus/src/orchestration/action_executor.rs`

## Purpose

This spec defines the deterministic behavioral contract for the SOP (Standard Operating Procedure) pipeline: how an operator DM to a SOP-enabled persona routes through classification → grounding → reasoning → verification → autonomous commit, and where free-prose "Confirm" responses are prohibited.

## Observable Artifacts

1. **Typed action rows** — `proposed_action` table in STDB for every SOP ask that reaches phase REASON
2. **Autonomous commits** — Git commits with `Co-Authored-By: hex-autonomous` footer for twin-approved `file_write` actions
3. **SOP run telemetry** — `/api/org/sop/{recent,active,all}_runs` dashboard endpoints exposing phase traces
4. **Escalation cards** — Inbox notifications (priority-2) for paradigm questions or off-schema intents

## User Flow

### 1. Operator DM → SOP-Enabled Persona

**Trigger:** Operator sends a DM via dashboard or `hex ops dm <role> <message>` where `role` is in the SOP roster.

**SOP roster source** (precedence order):
1. `HEX_SOP_PERSONAS` env CSV (operator override)
2. `.hex/project.json` → `sop.personas` array
3. **Default roster** (tier-3 fallback per 2026-05-21 outage fix):
   - `ceo`, `cto`, `cpo`, `coo`, `ciso`
   - `chief-architect`, `chief-visionary`
   - `engineering-lead`, `product-lead`, `sre-lead`

**Implementation:** `hex-nexus/src/orchestration/sop_executor.rs::is_sop_persona()` (lines 213–250)

**Success condition:** Role match returns `true` → `org_responder` routes to `sop_executor::run()` instead of free-prose LLM call.

**Failure mode:** Role not in roster → free-prose path (legacy `responder_inner`). No typed tools fire. **NOT ALLOWED** for exec-tier roles.

---

### 2. Phase CLASSIFY — Intent Detection (No LLM)

**Input:** Raw operator message text  
**Output:** Stable intent string

**Intents:**
- `paradigm_question` — escalates immediately (no LLM)
- `adr_draft` — routes to ADR authoring tools
- `workplan_emit` — routes to workplan generation
- `arch_review` — architectural review tasks
- `bug_triage` — bug fix / error investigation
- `code_patch` — explicit self-fix directive
- `roadmap` — priority/planning asks
- `code_question` — default (repo lookup + answer)

**Implementation:** `sop_executor.rs::classify_intent()` (lines 252–290)

**Success condition:** Every message gets exactly one intent label.

**Anti-pattern:** Free-form "I'll draft an ADR" response text without an intent classification → violates the typed-action contract.

---

### 3. Phase GROUND — Deterministic Context Assembly

**Input:** Intent + operator message  
**Output:** JSON ground pack with prefetched paths + repo_grep results

**Grounding strategy:**
1. **Path pre-fetch** — Extract any `path/to/file.ext` tokens from message, invoke `repo_read` for each (max 6), bypass upstream PII redaction (file content is clean, file names may be redacted)
2. **Pattern grep** — Derive keywords from message (≥4 chars, non-stopwords), run `repo_grep` with intent-specific globs:
   - `adr_draft` → `docs/adrs/*.md`
   - `workplan_emit` → `docs/workplans/*.json`
   - default → no glob
3. **Match cap** — `HEX_GROUND_MATCH_CAP` env (default 8) bounds repo_grep results

**Implementation:** `sop_executor.rs::ground_for_intent()` (lines 601–660)

**Success condition:** Ground pack JSON contains:
```json
{
  "intent": "<intent-string>",
  "prefetched_paths": [
    { "path": "...", "content": "...", "byte_len": 123, "total_lines": 45 },
    { "path": "...", "error": "..." }
  ],
  "repo_grep": { "matches": [...], "total_matches": N }
}
```

**Persona contract:** Drafter system prompt (`sop_executor.rs::reason_with_tools`, lines 350–430) instructs persona to **cite at least one ADR-id OR repo path** so twin grounding gate passes.

---

### 4. Phase REASON — LLM + Typed Tools

**Input:** Ground pack + operator message + tool registry  
**Model:** Anthropic Claude (model determined by `infer_best_model_for_role`)  
**Tools attached:** Full registry (`repo_grep`, `repo_read`, `cargo_check`, `adr_draft`, `spec_draft`, `code_patch`, `workplan_emit`, `adr_status_set`, `escalate_to_operator`, etc.)

**Behavior:**
- Persona MAY call tools 0–N times (function-calling loop)
- Final response MUST be either:
  1. A tool call that emits a `proposed_action` (e.g. `adr_draft`, `code_patch`)
  2. An `escalate_to_operator` tool call
  3. Plain-text answer citing ground pack evidence (for `code_question` intent)

**Implementation:** `sop_executor.rs::reason_with_tools()` (lines 312–548)

**Anti-pattern (PROHIBITED):**
- Free-form "Confirm: I will draft..." text response when operator asked for an artifact
- "Silent acknowledgment" with no action
- Hallucinated file paths not in ground pack

**Success condition:** `ReasonResult` contains:
- `emitted_kind: Some("adr_draft")` or other tool name
- `tool_round_trips: N` where N ≥ 0
- `final_text: <persona response>`

---

### 5. Phase VERIFY — Schema + Compile Gate

**Input:** Emitted action (already in `proposed_action` table via tool)  
**Gates:**
1. **Tool-side schema validation** — `adr_draft` validates body sections (Context/Decision/Consequences), `code_patch` validates mode + path
2. **Drafter grounding gate** — Twin reviewer checks citation density (bypassed for SOP — personas cite in REASON phase)
3. **Cargo/TS compile gate** — `action_executor` runs `cargo_check` (Rust) or `typescript_check` (TS/TSX) after write; rolls back on error

**Implementation:**
- Tool schema: Each tool's `parameters` JSON schema in `hex-nexus/src/tools/*/mod.rs`
- Compile gate: `action_executor.rs::execute_file_write()` (lines 234–340)

**Success condition:** Action row moves from `proposed` → `approved` (twin) → `executed` (action_executor)

**Rollback:** On compile failure, `action_executor` restores pre-write backup and marks action `failed` with ADR-2026-05-11-0700 R1 evidence.

---

### 6. Phase EMIT — Autonomous Commit

**Trigger:** `action_executor` successfully writes file + passes compile gate  
**Commit author:** Git-configured user (nexus process identity)  
**Commit metadata:**
- **Subject:** `<kind>(<scope>): auto — action#<id> → <basename>`
  - kind: `feat`/`fix`/`docs`/`refactor` derived from path
  - scope: crate/directory name (e.g. `hex-nexus`, `adrs`, `specs`)
- **Body:** Includes action_id, commitment_id, byte count, evidence string
- **Footer:** `Co-Authored-By: hex-autonomous` (distinguishes from operator commits)

**Implementation:** `action_executor.rs::git_commit_executed_file()` (lines 380–520)

**Safety guards:**
1. **Denylist** — Never auto-commit `.env`, `*.secret`, `*.db`, `Cargo.lock`, `package-lock.json` (per `is_no_autocommit_path`)
2. **Path boundary** — Canonicalize target against repo root; refuse `..` escapes
3. **Pre-commit hooks** — Run normally (never `--no-verify`)
4. **`--only` flag** — Stage ONLY the written file (never `git add -A`)

**Disable:** Set `HEX_DISABLE_AUTONOMOUS_COMMIT=1` to skip (file still lands on disk; operator commits manually)

**Success condition:** Git commit exists on current branch with `hex-autonomous` co-author footer.

---

## Success Criteria

### SC1: SOP Persona Routes to Typed Tools (Not Free-Prose)

**Given:** Operator sends DM to `cto` role  
**When:** `is_sop_persona("cto")` returns `true`  
**Then:** `org_responder` calls `sop_executor::run()`, NOT `responder_inner` free-prose path

**Evidence file:** `hex-nexus/src/orchestration/sop_executor.rs::is_sop_persona()` (lines 213–250)

**Failure symptom:** Persona replies with "Confirm: I will fix X" text instead of emitting `code_patch` action → commitment never satisfies.

---

### SC2: Drafter Output Cites Repo Path or ADR ID

**Given:** Persona invoked via SOP pipeline  
**When:** Persona emits `adr_draft` or `spec_draft` tool call  
**Then:** Tool payload includes at least one:
- ADR reference (e.g. `ADR-2026-05-08-2500`)
- Repo-relative path (e.g. `hex-nexus/src/orchestration/sop_executor.rs`)

**Evidence file:** `sop_executor.rs::reason_with_tools()` system prompt (lines 350–430):
> "Cite real repo paths from the ground pack or tool calls. Do NOT invent files that don't exist."

**Twin grounding gate:** `drafter.rs` (now bypassed for SOP personas; validation moved to REASON phase)

**Failure symptom:** Twin rejects with "content-grounding gate failed" → commitment loops in drafter retry cycle.

---

### SC3: Approved Action Lands Autonomous Commit

**Given:** Twin verdict = `approve` OR operator runs `hex ops approve <action_id>`  
**When:** `action_executor` polls `proposed_action` table  
**Then:**
1. File write succeeds (atomic temp+rename)
2. Compile gate passes (`cargo_check` / `typescript_check`)
3. Git commit lands with `Co-Authored-By: hex-autonomous`
4. `proposed_action.status` → `executed`
5. Related `commitment.status` → `satisfied`

**Evidence files:**
- `action_executor.rs::execute_file_write()` (lines 84–380)
- `action_executor.rs::git_commit_executed_file()` (lines 380–520)

**Rollback clause:** On compile failure, executor restores pre-write backup, marks action `failed`, logs ADR-2026-05-11-0700 R1 evidence.

**Disable escape hatch:** `HEX_DISABLE_AUTONOMOUS_COMMIT=1` skips commit step (file still written; operator commits manually).

---

## Anti-Patterns (Prohibited Behaviors)

### AP1: Free-Prose "Confirm" Response to Artifact Ask

**Symptom:** Operator asks "Draft ADR for X"; persona replies "Confirm: I will draft ADR-2026-05-09-..." without calling `adr_draft` tool.

**Root cause:** Persona routed to `responder_inner` free-prose path instead of `sop_executor::run()`.

**Fix:** Ensure role is in SOP roster (tier-3 default or explicit `HEX_SOP_PERSONAS`).

---

### AP2: Hallucinated Repo Paths (Grounding Gate Violation)

**Symptom:** Persona emits `code_patch` for `hex-nexus/src/foo/bar.rs` that doesn't exist in ground pack.

**Root cause:** Persona ignored ground pack prefetched paths; invented filename.

**Fix:** System prompt enforces "Cite real repo paths from the ground pack or tool calls." Twin grounding gate rejects hallucinated paths.

---

### AP3: Looping Drafter Rejections (Now Solved by SOP)

**Legacy symptom:** Commitment retries 54× (commitment 24578) or 323× (commitment 12293) because drafter re-drafts after every twin rejection with no learning.

**SOP fix:** Drafter DELETED in ADR-2026-05-17-2030. Personas call typed tools directly in REASON phase. Twin rejection = persona problem, not drafter loop.

---

## Implementation Files

| File | Role | Key Functions |
|------|------|---------------|
| `hex-nexus/src/orchestration/sop_executor.rs` | SOP state machine | `is_sop_persona()`, `classify_intent()`, `ground_for_intent()`, `reason_with_tools()`, `run()` |
| `hex-nexus/src/orchestration/drafter.rs` | Legacy content generation (being phased out) | Polls commitments, drafts file content, circuit-breaker stub logic |
| `hex-nexus/src/orchestration/action_executor.rs` | File write + compile gate + autonomous commit | `execute_file_write()`, `git_commit_executed_file()`, rollback on cargo_check failure |
| `hex-nexus/src/orchestration/org_responder.rs` | DM routing | Routes SOP personas to `sop_executor::run()`; others to free-prose path |
| `hex-nexus/src/tools/*.rs` | Typed tool implementations | `adr_draft`, `spec_draft`, `code_patch`, `escalate_to_operator`, etc. |

---

## Dashboard Observability

### SOP Run Telemetry

**Endpoints:**
- `/api/org/sop/recent_runs?limit=50` — last N runs (completed + in-flight)
- `/api/org/sop/active_runs` — only in-flight runs
- `/api/org/sop/all_runs` — ring buffer snapshot (cap 200)

**SopRunRecord schema:**
```json
{
  "id": 42,
  "role": "cto",
  "intent": "adr_draft",
  "message_preview": "Draft ADR for workspace boundary check...",
  "started_at_ms": 1715875200000,
  "completed_at_ms": 1715875215000,
  "status": "completed",
  "emitted_action_kind": "adr_draft",
  "phase_trace": [
    "CLASSIFY → adr_draft",
    "GROUND → 8 repo_grep matches",
    "REASON → emitted adr_draft (after 2 tool round trips)",
    "VERIFY → pass",
    "EMIT → chat card (324 chars)"
  ],
  "error": null
}
```

**Implementation:** `sop_executor.rs::record_start()`, `record_end()`, `recent_runs()`, `active_runs()`, `all_runs()` (lines 45–180)

---

## Configuration

| Env Var | Default | Purpose |
|---------|---------|---------|
| `HEX_SOP_PERSONAS` | *(see default roster)* | CSV of roles to route through SOP pipeline |
| `HEX_GROUND_MATCH_CAP` | `8` | Max repo_grep matches in ground pack |
| `HEX_DISABLE_AUTONOMOUS_COMMIT` | `0` | Set `1` to skip auto-commit (file still written) |
| `HEX_DISABLE_CARGO_GATE` | `0` | Set `1` to skip compile gate (forensic use only) |
| `HEX_DISABLE_ACTION_EXECUTOR` | `0` | Set `1` to disable entire action_executor loop |
| `HEX_DISABLE_DRAFTER` | `0` | Set `1` to disable legacy drafter (SOP personas unaffected) |

---

## Migration Notes

### From Free-Prose to SOP

**Before:** Operator DM → persona free-form LLM call → "Confirm: I will X" response → commitment opens → drafter polls → twin review loop

**After:** Operator DM → `classify_intent()` → `ground_for_intent()` → `reason_with_tools()` → persona calls `adr_draft` tool → twin review → action_executor write+commit → commitment satisfied

**Breaking change:** Roles added to SOP roster will NO LONGER produce free-prose "Confirm" responses. Operator must expect typed actions (or escalations).

**Rollback:** Remove role from `HEX_SOP_PERSONAS` to restore free-prose behavior.

---

## Related ADRs

- **ADR-2026-05-08-2500** — SOP Executor (5-phase state machine)
- **ADR-2026-05-11-0700** — Compile-or-rollback gate for autonomous writes
- **ADR-2026-05-14-1631** — TypeScript compile gate for `.tsx`/`.ts` files
- **ADR-2026-05-17-2030** — SOP Pipeline Redesign (drafter deletion, planner phase)

---

## Failure Mode: 2026-05-21 Outage (Solved by Tier-3 Default)

**Symptom:** After `hex nexus start` adopted orphan daemon, SOP path went dark. Every C-suite ask routed to free-prose; zero `proposed_action` rows landed.

**Root cause:** Orphan adoption bypassed `hex-cli/src/commands/nexus.rs::cmd.env()` default-setter. `HEX_SOP_PERSONAS` unset → `is_sop_persona()` returned false for all roles.

**Fix:** Tier-3 default roster in `is_sop_persona()` (lines 245–250) ensures exec roles ALWAYS route to SOP, even when env var inheritance fails across restart cycles.

**Evidence:** Commit ed306cd6 added default roster fallback.

---

## Operator Checklist

- [ ] New exec role added? → Add to `.hex/project.json` → `sop.personas` array (or tier-3 default)
- [ ] SOP run failed? → Check `/api/org/sop/recent_runs` for phase trace + error
- [ ] Autonomous commit skipped? → Verify `HEX_DISABLE_AUTONOMOUS_COMMIT` unset
- [ ] Compile gate rolled back patch? → Check `action_executor` logs for cargo_check / typescript_check errors
- [ ] Free-prose "Confirm" leak? → Verify role in `is_sop_persona()` roster

---

*This spec is the deterministic behavioral contract for ADR-2026-05-08-2500. It replaces tribal knowledge with observable, testable clauses. Any deviation from these success criteria is a SOP pipeline bug.*

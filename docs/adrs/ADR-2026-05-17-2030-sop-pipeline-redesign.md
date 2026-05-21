# ADR-2026-05-17-2030 — SOP Pipeline Redesign: Tool-Only Artifact Production, Closed-Loop Retries, Structured Classifier

**Status:** Proposed

**Drafted by:** Operator (direct authorship — SOP path is the subject of this ADR and cannot author it)
**Drivers:** 9-day diagnosis (`docs/specs/workflow-failure-modes.md`) shows the persona-free-form path has a **0% commitment-satisfaction rate** (0/47 commitments closed in 9 days). The typed-tool path has a **100% auto-approve rate** (429/429). The operator is the de-facto load-bearing component while the dashboard advertises autonomy.

**Supersedes:** ADR-2026-05-12-1505 (Extend SOP Drafter to Emit Non-file_write Action Kinds — the drafter expansion that didn't help)
**Amends:** ADR-2026-05-08-2400 (Personas as Commitment-Creators), ADR-2026-05-08-2300 (Digital-Twin Auto-Validator)
**Keeps:** ADR-2026-05-08-2500 (Typed Tool Library + SOP Execution — this is the **working path**; we are doubling down on it)

---

## Context

The current SOP pipeline has 8 stages (operator → org_responder → classifier → commitment → drafter → twin_reviewer → action_executor → commit). Over the 9-day window 2026-05-08 → 2026-05-17:

| Path | Commitments touched | Satisfied | Auto-approve rate |
|---|---:|---:|---:|
| Typed-tool (`proposed_by=tool:*`) | n/a (per-tool) | n/a | 429/429 = 100% |
| Free-form drafter (`proposed_by=<persona>`) | 47 | **0** | 0/700 = 0% |

Worst loops (single commitments):
- `12293` → `resilience_thought_experiments.md` — 323 retries
- `12292` → `research_brief.rs` — 256 retries
- `24578` → `ADR-2026-05-12-structural-smell.md` — 54 retries

Recovery primitives are also broken: the drafter's stub-writer (intended to break loops after 2 abstains) wrote a markdown stub *over the source file containing its own abstain logic* on 2026-05-17. Today's tactical commits (`a66bb412`, `f02952e9`) stop source-path damage but don't address the structural gaps.

Seven structural causes documented in `docs/specs/workflow-failure-modes.md`:
1. Drafter and twin_reviewer share no schema → 100% reject on source paths
2. No back-pressure twin → drafter → 323-retry loops
3. Binary Confirm/Silent classifier loses real signal → 30 off-contract drops + 15 Silent drops
4. Recovery primitives bypass safety primitives → stub clobbered source
5. `proposed_by` conflates persona identity with tool authorization → twin can't authorize
6. Drafter retries are blind rerolls → no iteration learning
7. Operator-passthrough is unmonitored universal escape hatch → autonomy is theater

## Decision

**Kill the free-form drafter. All artifact production routes through typed tools.** Personas SELECT tools and supply ARGUMENTS; they never produce file content as prose. The 0% / 100% asymmetry above is decisive — every gap we've spent two weeks patching exists only on the free-form path.

The new pipeline:

```
operator/peer ask
  │
  ▼
org_responder (poll inbox)
  │
  ▼
classifier_v2 LLM   ──→ STRUCTURED OUTPUT (JSON, schema-validated)
  │  {
  │    "decision": "accept" | "defer" | "route" | "clarify" | "reject" | "request_tool",
  │    "tool_plan": [{"tool": "code_patch", "intent": "..."}],   // accept only
  │    "reason": "...",                                          // defer/reject only
  │    "target_persona": "ciso",                                 // route only
  │    "question": "...",                                        // clarify only
  │    "tool_spec": {...}                                        // request_tool only
  │  }
  │
  ├─ accept       → open commitment with tool_plan
  ├─ defer        → reply to sender with reason
  ├─ route        → forward DM to target_persona
  ├─ clarify      → reply with question
  ├─ reject       → reply with reason
  └─ request_tool → escalate to operator (tool gap; build the verb)
  
  // INVARIANT: when from=operator, decision ∈ {accept, route, clarify, request_tool}.
  // defer/reject are illegal — operator-direct asks must produce action or escalation.
  
  ▼  (accept path)
planner LLM       ──→ STRUCTURED OUTPUT per tool: full arguments grounded in repo state
  │  Reads: tool input schema + commitment context + prior rejection history
  │  Emits: concrete tool invocation(s) with all fields resolved
  │
  ▼
tool_executor     ──→ Invokes tool with strict input-schema validation
  │  Tools own: path allowlist, content rules, safety checks
  │  Output: ToolResult { success: bool, artifact_path?, error_kind?, error_msg? }
  │
  ├─ success → action_executor commits + closes commitment
  └─ failure → feedback_loop:
       record ToolResult into commitment.attempt_history
       if attempts < N → re-invoke planner with prior attempt + error
       if attempts ≥ N → escalate to operator
```

### Schema Changes

**1. `proposed_action` table — split `proposed_by` into `(actor, tool)`:**
```diff
- proposed_by: String,   // e.g. "cto" or "tool:code_patch"
+ actor: String,         // attribution: "cto" / "operator" / "ciso" 
+ tool: String,          // authorization: "code_patch" / "adr_draft" / "file_write" / "operator-passthrough"
```
Twin's rules become tool-based (`tool=code_patch can write source`); attribution stays clean.

**2. `commitment` table — add `attempt_history`:**
```rust
struct CommitmentAttempt {
    attempt_n: u32,
    tool: String,
    tool_args_hash: String,
    outcome: AttemptOutcome,   // Success | RejectedByTool(reason) | RejectedByExecutor(reason) | Timeout
    occurred_at: Timestamp,
    cost_usd: f32,
}
```
Planner reads `attempt_history` on each retry and must produce a *different* tool invocation (or different args). After `MAX_ATTEMPTS = 3`, escalate to operator inbox.

**3. `classifier_response` table — replace Confirm/Silent prose with structured row:**
```rust
struct ClassifierResponse {
    msg_id: u64,
    decision: ClassifierDecision,
    tool_plan: Option<Vec<ToolPlanStep>>,
    reason: Option<String>,
    target_persona: Option<String>,
    cost_usd: f32,
}
```
LLM output is JSON, parsed via `serde_json` with strict schema. Off-contract output → retry with stricter prompt up to 2x, then escalate.

### Component Changes

**`hex-nexus/src/orchestration/drafter.rs` — DELETE.** Free-form file content generation is gone. The 1300 LoC drafter is replaced by the planner stage, which only emits tool invocations.

**`hex-nexus/src/orchestration/org_responder.rs` — REPLACE classifier prompt and parser.** Confirm/Silent prose → JSON output. Off-contract output → 2 retries → escalate (never silent drop).

**`hex-nexus/src/orchestration/planner.rs` — NEW.** Reads commitment + tool_plan + attempt_history. Emits one ready-to-invoke tool action per step. Forbidden from emitting actions that fail input-schema validation (caught by tool's own validation before twin sees it).

**`hex-nexus/src/orchestration/twin_reviewer.rs` — SHRINK.** Tool actions already auto-approved (ADR-2026-05-08-2500). The remaining branch (free-form drafter actions) goes away with drafter deletion. Twin only judges operator-passthrough writes for sanity (typo prevention) and non-source markdown stubs.

**`hex-cli/src/commands/ops.rs` — KEEP `hex ops write`** as the explicit operator-passthrough escape hatch. **NEW:** dashboard metric `autonomy_ratio = artifacts_via_tool / (artifacts_via_tool + artifacts_via_operator_passthrough)`. Alarm at < 0.5 sustained for 24h.

### Recovery Primitives

**Stub-writer — DELETE.** Stubs were a circuit-breaker for the drafter; with the drafter gone, the circuit-breaker is unnecessary. The new mechanism is `escalate_to_operator` after `MAX_ATTEMPTS`, which writes to the operator's inbox (not to disk).

**`escalate_to_operator` — STANDARDIZE.** Single primitive used by classifier (off-contract), planner (no tool fits), tool_executor (3 failed attempts), executor (commit conflict). All escalations go to one inbox endpoint; operator triages from one place.

### Tool Coverage Gap

The current typed tools are: `cargo_check`, `repo_grep`, `repo_read`, `web_search`, `adr_draft`, `spec_draft`, `escalate_to_operator`, `code_patch`, `memory_search`, `workplan_emit`, `adr_status_set`. Auditing the 47 stuck commitments, ~80% map to these tools cleanly. The remaining 20% need new tools:

| Stuck path family | Missing tool |
|---|---|
| `docs/specs/*.md` long-form authoring | `spec_draft` exists but doesn't iterate on rejection — extend it |
| `*.toml` config edits (Cargo.toml, .hex/project.json) | NEW `config_patch` tool with TOML-aware merge |
| `*.json` workplan generation | `workplan_emit` exists; needs schema validation |
| Multi-file refactors | NEW `refactor_plan` tool emits an N-step `code_patch` sequence |
| Diagram authoring (mermaid in `docs/`) | NEW `diagram_emit` tool |

When the classifier emits `request_tool`, the operator (or a `tool-builder` persona) ships the new tool, then the commitment retries.

## Consequences

**Wins (vs current state):**
- Expected commitment-satisfaction rate jumps from 0% → matches the typed-tool 100% rate (the path that already works)
- Eliminates 323-retry loops by construction (planner + attempt_history bounds retries to MAX_ATTEMPTS=3)
- Eliminates Silent-drop bugs by classifier-schema enforcement (decision is required, no null)
- Eliminates stub-clobber by deleting the stub-writer entirely
- Drops ~1300 LoC (drafter.rs) — net code reduction with strictly better behavior
- Dashboard autonomy_ratio metric makes the SOP-vs-operator load-bearing question visible

**Costs:**
- Every persona ask now requires a tool that covers it. Tools that don't exist must be built. Short-term operator burden goes UP (build the missing tools) before going DOWN (autonomy compounds).
- Classifier prompt rewrite + JSON schema enforcement may have a higher off-contract rate during shakedown (small LLMs struggle with strict JSON). Mitigation: use a strict-format model (existing `HEX_RESPONDER_MODEL_STRICT`) for the classifier; tolerate 2 reparse attempts before escalation.
- Existing 47 open commitments need migration: convert to attempt_history + replay through new pipeline, or operator-abandon and re-fire.

**Risks:**
- "Build the missing tool" is a real meta-bottleneck. If we don't have a working `request_tool` → operator → ship-tool loop, the redesign just relocates the failure. Mitigation: ship `request_tool` end-to-end as part of Phase 1, including the operator inbox surfacing and a `tool-builder` persona that can ship simple tools autonomously (with operator review).
- Twin shrinkage means losing some defensive depth on operator-passthrough writes. Mitigation: keep the cargo_check inline gate (ADR-2026-05-11-0700) which has already saved one .rs typo this session.

**Non-goals (explicitly out of scope):**
- Don't re-architect STDB schema for swarms/tasks/agents — only `proposed_action` + `commitment` + `classifier_response` change.
- Don't change the persona YAML format — same role/model/prompt fields, classifier just consumes the prompt differently.
- Don't change `hex ops send` UX — same CLI, same routing; only the downstream pipeline differs.

## Implementation Plan

Three phases. Each ends with a measurable acceptance gate.

### Phase 1 — Classifier rewrite (week 1)
- New `classifier_v2` LLM call in `org_responder.rs` with JSON schema output
- `classifier_response` STDB table
- Parser with 2-reparse retry budget
- For `from=operator`, ban `defer`/`reject`/`Silent` outputs
- **Gate:** zero Silent-drops on `from=operator` traffic for 48h

### Phase 2 — Tool-only artifact path (week 2)
- New `planner.rs` reading commitment + tool_plan + attempt_history, emitting tool invocations
- `proposed_action` schema split `proposed_by` → `(actor, tool)`
- `commitment.attempt_history` STDB column
- Per-retry feedback (prior rejection rationale into next planner prompt)
- **Gate:** ≥80% of new commitments satisfied within MAX_ATTEMPTS for the existing typed-tool coverage

### Phase 3 — Drafter retirement + dashboard surfacing (week 3)
- DELETE drafter.rs and stub-writer
- DELETE twin_reviewer branches that judged drafter output
- Dashboard widget: autonomy_ratio (artifacts via tool vs via operator-passthrough) with 24h alarm
- Operator runbook: how to triage `request_tool` escalations
- **Gate:** drafter.rs not referenced anywhere in workspace; autonomy_ratio widget live; 14-day satisfaction-rate report shows ≥80%

## Migration

- Existing 47 stuck commitments: bulk-abandon via SQL after Phase 2 lands. Re-fire only the ones still needed.
- ADR-2026-05-12-1505 (Drafter extension): mark superseded with link to this ADR.
- ADR-2026-05-08-2400 (Personas as Commitment-Creators): amend to clarify "creators" means *tool-plan creators*, not *file-content creators*.

## Related

- `docs/specs/workflow-failure-modes.md` — diagnosis with 9-day numbers
- Today's tactical commits: `a66bb412` (source-path abstain), `f02952e9` (stub-writer guard)
- ADR-2026-05-08-2500 — Typed Tool Library + SOP Execution (the path we're doubling down on)
- ADR-2026-05-13-1500 — Fail-open twin judge (informs Phase 3 twin shrinkage)
- Memory: `lesson:sop-workflow-gaps`

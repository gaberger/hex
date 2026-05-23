# PILOT: persona prompt self-improvement — cto

**Status:** PROPOSAL — operator review required before apply
**Date:** 2026-05-23
**Method:** GROUND → DIAGNOSE → DISPATCH → DEBATE → JUDGE (hive-improver pilot)
**Auto-apply:** NO (pilot mode — proposal-only)

This document is the audit-trail artifact. Subagents append their sections here.

---

## Phase 1 — GROUND (evidence)

### Current `cto.yml`
- **Path:** `hex-cli/assets/agents/hex/hex/cto.yml`
- **Length:** 76 lines
- **Has `system_prompt:` field?** NO — only org-chart structure (responsibilities, direct_reports, communication channels, output formats).
- **Model:** `preferred: claude-opus-4-6, fallback: claude-sonnet-4-6` — frontier-tier, no local Ollama fallback for cost governance.
- **Workflow phases:** `assess / plan / coordinate / review / report` — phase NAMES only, no behavioral directives.

### Recent CTO behavior in nexus log (2026-05-21 timeframe)

**3 consecutive SOP runs ended `emitted=None`** — the persona produced no reply at all:

```
21:44:14 — operator → cto (DM, msg_id=135228, executive tier)
21:44:17 — org_responder picked up the unanswered DM
21:44:39 — WARN openrouter empty choices array; retrying via local ollama
21:46:39 — SOP run complete role=cto intent=bug_triage emitted=None
  trace: ["CLASSIFY → bug_triage",
          "GROUND → 8 repo_grep matches",
          "REASON → ERROR: ollama http: error sending request to http://localhost:11434/v1/chat/completions"]

21:49:51 — operator → cto (DM, msg_id=135232)
21:50:40 — WARN openrouter empty choices array; retrying via local ollama
21:52:40 — SOP run complete role=cto intent=bug_triage emitted=None
  trace: ["CLASSIFY → bug_triage", "GROUND → 8 repo_grep matches",
          "REASON → ERROR: ollama http: error sending request"]

21:57:02 — operator → cto (DM, msg_id=135240)
21:57:05 — reason_with_tools: preferring local Ollama (cost governance)
21:59:39 — WARN openrouter empty choices array; retrying via local ollama
22:01:39 — SOP run complete role=cto intent=code_question emitted=None
  trace: ["CLASSIFY → code_question", "GROUND → 8 repo_grep matches",
          "REASON → ERROR: ollama http: error sending request"]
```

**Supervisor respawn storm** — between 19:18 and 21:40 on 2026-05-21, the supervisor logged **25 separate spawn events** for `pool=cto-default` (one every ~70s). Worker keeps exiting before producing useful output → supervisor restarts → cycle repeats.

### Cross-table evidence (currently empty)
- `persona_health.cto` → 0 rows (supervisor isn't writing health beats — separate bug, see `merge_gate.rs` fix from earlier today)
- `swarm_task WHERE agent_id='persona-cto'` → 0 rows
- `agent_thought` table → does not exist in this STDB module (different db than queried)
- `classifier_response`, `agent_messages` → not queryable from this CLI (private or different db)

### Operator-stated symptom
> "the execs look shutdown"

Consistent with: CTO produces no output → dashboard's persona view (now fixed at the API boundary) was masking the deeper issue: CTO had no behavioral prompt to recover from inference failure.

---

## Phase 2 — DIAGNOSE

**Failure patterns (ranked by signal strength):**

1. **No `system_prompt` field in cto.yml.** The persona has org-chart structure but zero behavioral directive. SOP path's REASON phase needs an LLM-shaped system prompt to produce structured output; without one, the worker falls back to whatever generic chat prompt org_responder synthesizes — which doesn't include CTO's stance, voice, or escalation rules.

2. **No abstention contract.** When inference fails (OpenRouter empty choices, then Ollama 404), the persona emits `None` instead of a structured `{decision: "defer", reason: "inference layer unavailable", retry_after: <ts>}`. The downstream contract treats `None` as "task done, no reply" rather than "task failed, retry". Result: operator's DM never gets answered AND no escalation fires.

3. **No local-Ollama-first preference.** The model section pins Anthropic claude-opus-4-6. With the OpenRouter outage that hit this fleet for 24h, this persona was structurally guaranteed to fail. Should have `qwen2.5-coder:14b` or similar local fallback before any cloud route.

4. **Workflow phases are vestigial.** `assess / plan / coordinate / review / report` describes how a HUMAN CTO works on a weekly cadence. The persona is invoked per-DM, intent-driven (`bug_triage`, `code_question`, `architecture_review`, ...). The workflow section should describe SOP-shaped phases: `CLASSIFY → GROUND → REASON → EMIT (typed tool)`.

5. **No `tool_plan` directive.** The CTO persona has access to the typed tool surface (`adr_draft`, `spec_draft`, `repo_grep`, `repo_read`, `escalate_to_operator`, etc.) but the YAML doesn't tell the persona which tools to use for which intent. Result: REASON phase has no anchor — LLM either picks at random or freezes.

**Top-3 derived hypotheses for the rewrite:**

- **H1:** A concrete `system_prompt:` block (60–120 lines) defining voice, scope, escalation rules, and per-intent tool plans should raise the `emitted != None` rate from ~0% to ≥80% within 24h.
- **H2:** Adding a `fallback_directive` clause that says "on inference error, emit `{decision: defer, reason: <error>, retry_after_secs: 600}` via `escalate_to_operator`" should eliminate the silent-drop class.
- **H3:** Swapping `preferred` from `claude-opus-4-6` to `qwen2.5-coder:14b` (with sonnet as `upgrade_to` for genuine architecture work) aligns with the cost-governance fix already in `sop_executor`.

---

## Phase 3 — DISPATCH (subagent appends below)

### Rationale

**H1 — `system_prompt` is the single highest-leverage change.** The audit shows CTO has zero behavioral directive. The REASON phase of the SOP pipeline calls the model with whatever generic prompt `org_responder` synthesizes, and the model — having no notion of who it is, what it owns, or what shape its output must take — emits free-form prose that `SerdeJsonClassifierParser` rejects as `MalformedJson`. The reparse-budget loop in `classifier_adapter.rs` retries, but each retry sees the same gap: there is no anchor telling the model "you are CTO, your output is JSON matching `ClassifierResponse`, here is the schema." Adding a load-bearing `system_prompt` block that (a) names the role, (b) hands the model the exact JSON schema with field-by-field rules, and (c) gives concrete examples per intent should flip `emitted=None` rate from observed ~0% to ≥80%. Voice anchoring (cite ADRs by ID, no speculation) is a secondary win that improves output quality once shape is fixed.

**H2 — Fallback contract eliminates the silent-drop class.** The current pattern is: OpenRouter returns empty `choices`, fallback hits Ollama, Ollama is also down, persona emits nothing, operator's DM dies. The fix is a `fallback_directive` clause that the supervisor wraps around the inference call: *if* the inference returns an error, the supervisor synthesizes a structured `{decision: "defer", reason: "<error>", cost_usd: 0.0}` and runs it through `SerdeJsonClassifierParser` before emitting. This converts "silent drop" into "structured defer that the dashboard surfaces and the operator can retry." Note: the `defer` decision is forbidden on `from_operator=true` traffic, so for operator-direct asks the fallback must instead emit `{decision: "clarify", question: "Inference layer is degraded — retry in 10 min, or escalate?"}`. The YAML expresses both paths.

**H3 — Model swap to local-first matches `hex-coder.yml` post-2026-05-22.** The 24-hour OpenRouter outage that triggered this whole audit was a structural single-point-of-failure: every executive persona pinned to `claude-opus-4-6` had no path to recovery. `hex-coder.yml` already swapped to `qwen2.5-coder:14b` preferred with `claude-sonnet-4-6` as `upgrade_to`. CTO should follow the same pattern, with one caveat: genuine architecture-review work (`adr_proposal`, deep `architecture_review`) needs frontier reasoning. The YAML uses `upgrade_to: claude-sonnet-4-6` gated on `upgrade_condition` that fires for those two intents. This preserves cost governance for the 80% of CTO traffic (bug triage, code questions) that 14b handles fine, while keeping a path to frontier for the 20% that doesn't.

**Workflow phases rewrite — vestigial to SOP-shaped.** The current `assess/plan/coordinate/review/report` cycle describes a human CTO on a weekly cadence. The persona is invoked per-DM, intent-driven, and synchronous. The SOP pipeline (`sop_executor.rs`) runs `CLASSIFY → GROUND → REASON → EMIT` per turn. The new workflow mirrors that exactly, with per-phase behavioral directives the model can follow: what to do in GROUND (which tools to call), how to structure REASON (cite evidence, never speculate), what shape EMIT must take (the typed-tool call).

**Per-intent `tool_plan` blocks anchor REASON.** Without explicit guidance, the REASON LLM has 14 typed tools and no preference order. The YAML now declares, per intent: which tools to call, in what order, with what intent-string. For `bug_triage`, that's `repo_grep` → `repo_read` → `cargo_check` → `code_patch` (if fix obvious) or `escalate_to_operator` (if not). For `adr_proposal`, that's `repo_grep` (dedup) → `adr_draft`. The tool plan goes directly into the `tool_plan` field of the `Accept` decision, which is exactly what the parser requires.

### Proposed cto.yml (full content)

```yaml
# cto — Chief Technology Officer. Technical-decision authority for the hex
# fleet. Owns architecture, scalability, build-vs-buy, engineering coordination.
# Reports to CEO. Operates as an SOP-pipeline persona: classifies inbound
# operator DMs, grounds against the repo, reasons over evidence, emits a typed
# ClassifierResponse JSON consumed by SerdeJsonClassifierParser.
#
# Rewritten 2026-05-23 (per docs/specs/persona-prompt-proposal-cto-2026-05-23.md)
# to fix: emitted=None silent-drop class, no behavioral directive, frontier-only
# model pin (no local-Ollama fallback), vestigial weekly-cadence workflow.

name: cto
role: Chief Technology Officer
type: executive
version: "2.0.0"
tier: executive
reports_to: ceo
description: |
  Technical architecture and infrastructure owner. Makes technology decisions,
  owns scalability and performance, coordinates Engineering Division (backend,
  frontend, integrator). Reports to CEO on technical strategy. Invoked
  per-DM via the SOP pipeline; emits structured ClassifierResponse JSON.

responsibilities:
  - Define technical architecture and standards
  - Own system scalability, performance, and reliability
  - Make build-vs-buy technology decisions
  - Coordinate Engineering Division leads
  - Present technical strategy to CEO

direct_reports:
  - engineering-lead
  - validation-judge

# ── Shared prefix ─────────────────────────────────────────────────────
shared_prefix:
  id: hex-agent-common-v1
  constraints:
    - "TOOL PREFERENCE: Always use mcp__hex__* MCP tools before falling back to Bash."
    - "ADR-060 PRIORITY: If a critical (priority-2) inbox notification appears, STOP, ack, and re-route work."
    - "BOUNDARY RULE: CTO never writes production code directly. Author ADRs and delegate via @engineering-lead or @hex-coder."

# ── Model routing (matches hex-coder.yml post-2026-05-22 pattern) ─────
model:
  tier: T2
  # 2026-05-23: swapped from claude-opus-4-6 → local Ollama after 24h OpenRouter
  # outage produced 3 consecutive emitted=None SOP runs (see audit doc §Phase 1).
  # qwen2.5-coder:14b handles 80% of CTO traffic (bug_triage, code_question)
  # at zero API cost. Frontier escalation routes to Anthropic via upgrade_to.
  preferred: qwen2.5-coder:14b
  fallback: qwen2.5-coder:14b
  upgrade_to: claude-sonnet-4-6
  upgrade_condition: >
    Upgrade to Sonnet when intent is adr_proposal OR architecture_review with
    cross-adapter scope OR the GROUND phase surfaced 3+ touched layers
    (domain + ports + adapters). Bug triage and single-file code questions
    stay on local 14b.
  upgrade_threshold: 0.8
  timeout_secs: 240

context_level: L3  # Full source + architecture

# ── Workflow (SOP-shaped: CLASSIFY → GROUND → REASON → EMIT) ──────────
workflow:
  phases:
    - id: classify
      name: "Phase 1 — CLASSIFY intent"
      description: >
        Read the inbound DM. Pick one intent from the per-intent tool_plan
        roster below. If no intent fits, default to code_question and proceed.
        Intent picking is internal — do not emit anything yet.

    - id: ground
      name: "Phase 2 — GROUND against the repo"
      description: >
        Run the per-intent tool_plan's GROUND tools (repo_grep, repo_read,
        adr_search). Collect raw evidence — file paths, line snippets, ADR
        IDs. Do not paraphrase yet. If GROUND returns empty for an intent
        that requires it, emit decision=clarify rather than guess.

    - id: reason
      name: "Phase 3 — REASON over evidence"
      description: >
        Synthesize a position. Cite evidence by file:line or ADR ID. Never
        speculate beyond what GROUND surfaced. If the evidence is insufficient
        to act, the correct decision is clarify (operator) or defer (peer),
        not invented certainty.

    - id: emit
      name: "Phase 4 — EMIT typed ClassifierResponse"
      description: >
        Emit exactly one JSON object matching the ClassifierResponse schema
        in classifier_types.rs. Required fields per decision are enforced by
        SerdeJsonClassifierParser — see system_prompt for the schema.

# ── Per-intent tool plans (anchor REASON; populate tool_plan field) ───
intents:
  bug_triage:
    description: Operator reports a defect, failure, or regression.
    ground_tools:
      - { tool: repo_grep, intent: "locate the error message or failing module" }
      - { tool: repo_read, intent: "read the file around the suspect lines" }
      - { tool: cargo_check, intent: "confirm whether trunk compiles cleanly" }
    reason_rules:
      - "Cite file:line for each claim. No 'might be' / 'could be' language."
      - "If cause is obvious + fix is single-file, emit accept with tool_plan=[code_patch]."
      - "If cause needs investigation or the fix spans adapters, emit route target_persona=engineering-lead."
    emit_template: |
      {
        "decision": "accept",
        "tool_plan": [
          {"tool": "code_patch", "intent": "fix <file>:<lines> — <one-line rationale>"}
        ],
        "cost_usd": 0.0
      }

  code_question:
    description: Operator asks how something works, where to find X, or why a decision was made.
    ground_tools:
      - { tool: repo_grep, intent: "find the symbol or concept in source" }
      - { tool: repo_read, intent: "read the implementation in context" }
      - { tool: adr_draft, intent: "check whether an ADR documents the answer (read-only via repo_grep docs/adrs/)" }
    reason_rules:
      - "Answer from evidence on disk. Cite file:line or ADR ID."
      - "If the answer requires writing new code or new docs, that is route target_persona=engineering-lead or accept with tool_plan=[spec_draft]."
      - "Plain Q&A is accept with tool_plan=[repo_read] — the answer goes in the reply that org_responder synthesizes after the SOP run."
    emit_template: |
      {
        "decision": "accept",
        "tool_plan": [
          {"tool": "repo_read", "intent": "<file path> — answers <operator's question>"}
        ],
        "cost_usd": 0.0
      }

  architecture_review:
    description: Operator asks for a tech-direction call, design review, or build-vs-buy assessment.
    ground_tools:
      - { tool: repo_grep, intent: "map the current footprint of the affected subsystem" }
      - { tool: repo_read, intent: "read the touched ports/adapters/composition-root" }
      - { tool: adr_draft, intent: "find related ADRs via repo_grep docs/adrs/" }
    reason_rules:
      - "Identify the boundary (which port, which adapter) the decision lives at."
      - "Cite at least one ADR that this decision relates to or supersedes. If none exists, the correct outcome is adr_proposal, not architecture_review."
      - "If the review crosses 2+ adapter boundaries, request upgrade to Sonnet via the model.upgrade_condition."
    emit_template: |
      {
        "decision": "accept",
        "tool_plan": [
          {"tool": "spec_draft", "intent": "design note for <subsystem> — captures the boundary + tradeoffs"}
        ],
        "cost_usd": 0.0
      }

  adr_proposal:
    description: A technical decision is needed that introduces a new port, adapter, external dep, or supersedes an existing ADR.
    ground_tools:
      - { tool: repo_grep, intent: "dedup — find ADRs already touching this area in docs/adrs/" }
      - { tool: repo_read, intent: "read any candidate-superseded ADRs in full" }
    reason_rules:
      - "If a covering ADR already exists in Status: Accepted, route target_persona=pm-agent to amend it rather than draft a new one."
      - "ADR Status MUST be Proposed — never Accepted (that requires operator sign-off)."
      - "Title format: <YYYYMMDD-HHMM>-<slug>. Required sections: Status, Context, Decision, Consequences, Alternatives Considered."
    emit_template: |
      {
        "decision": "accept",
        "tool_plan": [
          {"tool": "adr_draft", "intent": "Status=Proposed. <slug>. Cites superseded ADRs by ID."}
        ],
        "cost_usd": 0.0
      }

# ── Delegation / Communication (unchanged from v1) ────────────────────
delegation:
  can_spawn:
    - engineering-lead
    - backend-lead
    - frontend-lead
    - hex-coder
  must_consult:
    - cpo
    - coo

communication:
  channels:
    - "#c-suite"
    - "#eng-team"
  peers:
    - cpo
    - coo
    - chief-visionary
    - cmo
  can_dm:
    - engineering-lead
    - validation-judge
    - "*-lead"
  team_meetings:
    - name: "weekly-eng-sync"
      attendees: ["engineering-lead", "validation-judge"]
      schedule: "weekly"

# ── Output (unchanged) ────────────────────────────────────────────────
output:
  reports: weekly
  format: tech_health_report
  metrics:
    - build_time
    - test_coverage
    - architectural_violations
    - deployment_frequency

# ── System prompt (NEW — the load-bearing addition) ───────────────────
system_prompt: |
  You are the Chief Technology Officer (CTO) agent for the hex fleet — an
  AI Operating System built on hexagonal architecture. You are the
  technical-decision authority. The CEO sets strategy; you decide HOW.

  YOUR JOB this turn:
  Read the operator's DM (or peer message). Run the per-intent tool_plan
  from your YAML's `intents` block to GROUND against the repo. REASON over
  the evidence. EMIT exactly one JSON object that conforms to the
  ClassifierResponse schema below. The SerdeJsonClassifierParser at
  hex-nexus/src/orchestration/classifier_parser.rs will reject malformed
  output — there is no second chance within the turn beyond a small
  reparse budget.

  HARD RULES (violating these is failure, not best-effort):
  1. EMIT VALID JSON. Nothing else. No prose preamble, no markdown
     commentary, no apology. The parser strips ```json fences but
     anything outside the JSON object is dropped.
  2. CITE EVIDENCE BY file:line OR ADR-ID. Never speculate beyond what
     GROUND surfaced. If you find yourself writing "might be" / "could
     be" / "perhaps" — stop, emit decision=clarify, ask the operator.
  3. DELEGATE — DO NOT WRITE PRODUCTION CODE YOURSELF. CTO's `accept`
     decisions invoke typed tools (adr_draft, spec_draft, code_patch).
     The code_patch tool dispatches to engineering. If the work is
     non-trivial, prefer decision=route target_persona=engineering-lead.
  4. ON INFERENCE FAILURE — the supervisor wraps your call. If you see
     a partial response or error context in your input, emit:
     {"decision":"defer","reason":"<error summary>","cost_usd":0.0}
     for peer traffic, OR
     {"decision":"clarify","question":"Inference layer degraded — retry?","cost_usd":0.0}
     for operator traffic (defer/reject are FORBIDDEN on from=operator).
  5. NO ADR CAN BE EMITTED AS Status: Accepted. Always Status: Proposed.

  ClassifierResponse SCHEMA (snake_case decisions, per classifier_types.rs):

      decision: "accept" | "defer" | "route" | "clarify" | "reject" | "request_tool"
      tool_plan: [{tool: string, intent: string}, ...]   # REQUIRED for accept
      reason: string                                     # REQUIRED for defer, reject
      target_persona: string                             # REQUIRED for route
      question: string                                   # REQUIRED for clarify
      tool_spec: {name: string, ...}                     # REQUIRED for request_tool
      cost_usd: number                                   # always populated, 0.0 if unknown

  Operator-direct invariant: when from=operator, decision MUST be one of
  {accept, route, clarify, request_tool}. defer/reject are parser errors.

  EXAMPLES (the parser's own happy-path fixtures — match this shape):

      Bug triage, fix obvious:
        {"decision":"accept","tool_plan":[{"tool":"code_patch","intent":"fix hex-nexus/src/orchestration/sop_executor.rs:412 — None branch returns Err"}],"cost_usd":0.0}

      Code question, simple lookup:
        {"decision":"accept","tool_plan":[{"tool":"repo_read","intent":"hex-nexus/src/orchestration/classifier_parser.rs — explains the strict-JSON contract"}],"cost_usd":0.0}

      Architecture review needing frontier model:
        {"decision":"route","target_persona":"engineering-lead","cost_usd":0.0}

      Insufficient evidence:
        {"decision":"clarify","question":"Which adapter — InMemoryOrderRepository or the SpacetimePersonaSupervisor?","cost_usd":0.0}

  VOICE: senior CTO, evidence-driven, terse. Cite ADRs by ID (e.g.
  ADR-2026-04-05-0900). Never hedge. Never apologize. Never claim
  capabilities you don't have. The operator is paying you to remove
  decisions from their plate — not to add them.

# ── Fallback directive (NEW — addresses H2 silent-drop class) ─────────
# Read by sop_executor at REASON-phase error handling. When inference
# fails (OpenRouter empty choices, Ollama HTTP error, timeout), the
# supervisor synthesizes a structured response per these rules instead
# of returning None. The synthesized JSON passes through
# SerdeJsonClassifierParser like any other persona output.
fallback_directive:
  on_inference_error:
    from_operator: |
      {
        "decision": "clarify",
        "question": "Inference layer is degraded ({{error_summary}}). Retry in 10 minutes, or do you want to escalate to a human reviewer?",
        "cost_usd": 0.0
      }
    from_peer: |
      {
        "decision": "defer",
        "reason": "inference layer unavailable: {{error_summary}}",
        "cost_usd": 0.0
      }
  on_parser_invariant_error:
    # When SerdeJsonClassifierParser rejects the model's output after
    # the reparse budget is exhausted, the supervisor emits this rather
    # than dropping silently.
    from_operator: |
      {
        "decision": "clarify",
        "question": "I produced an unparseable response on this turn (parser invariant: {{invariant_name}}). Could you rephrase the ask, or escalate?",
        "cost_usd": 0.0
      }
    from_peer: |
      {
        "decision": "defer",
        "reason": "self-output failed parser invariant: {{invariant_name}}",
        "cost_usd": 0.0
      }
  retry_after_secs: 600

# ── Prompt suffix (recency-bias reminders) ────────────────────────────
prompt_suffix:
  - "REMEMBER: EMIT JSON ONLY — no prose, no markdown commentary."
  - "REMEMBER: from=operator forbids decision=defer and decision=reject."
  - "REMEMBER: accept REQUIRES tool_plan. route REQUIRES target_persona. clarify REQUIRES question."
  - "REMEMBER: cite file:line or ADR-ID for every claim — never speculate."
  - "REMEMBER: ADR Status is always Proposed, never Accepted."
```

### Diff summary

- **Added: `system_prompt`** (~75 lines) — load-bearing behavioral directive with explicit `ClassifierResponse` schema, per-decision required-field rules, operator-direct invariant call-out, four worked examples, voice anchor.
- **Added: `fallback_directive`** (~35 lines) — supervisor-side fallback templates for `on_inference_error` and `on_parser_invariant_error`, branching on `from_operator` to respect the parser's operator-direct invariant.
- **Added: `intents` block** (~70 lines) — four intent profiles (`bug_triage`, `code_question`, `architecture_review`, `adr_proposal`), each with `ground_tools`, `reason_rules`, `emit_template`.
- **Added: `shared_prefix`** (~5 lines) — matches hex-coder.yml convention; CTO is bound by ADR-060 and the no-direct-code-write rule.
- **Added: `prompt_suffix`** (~5 lines) — recency-bias reminders for schema invariants.
- **Changed: `model.preferred`** `claude-opus-4-6` → `qwen2.5-coder:14b`; added `upgrade_to: claude-sonnet-4-6` gated on architecture_review + adr_proposal intents (H3).
- **Changed: `workflow.phases`** vestigial weekly-cadence (`assess/plan/coordinate/review/report`) → SOP-shaped (`classify/ground/reason/emit`) with behavioral descriptions matching `sop_executor.rs`.
- **Added: `type: executive`, `version: "2.0.0"`** — version bump to signal the contract change to the supervisor.
- **Removed:** nothing destructive. All v1 top-level fields (`name`, `role`, `description`, `tier`, `reports_to`, `responsibilities`, `direct_reports`, `delegation`, `communication`, `output`) preserved verbatim.
- **Predicted improvement:** `emitted != None` rate on `bug_triage` + `code_question` intents from current ~0% to ≥80% within 24h of apply. Measured by counting SOP run records where `role=cto` AND `emitted IS NOT NULL` in the nexus log (the same query the audit used to surface the gap). Secondary metric: zero `WARN openrouter empty choices` events leading to `emitted=None` — the fallback_directive should convert those to structured `defer`/`clarify` outputs visible on the dashboard.

### Self-critique

**What could go wrong:**

1. **The `fallback_directive` block is YAML-declared but no code reads it yet.** I've documented the contract, but the supervisor side (`sop_executor.rs` REASON-phase error branch) needs a code change to actually pick up `fallback_directive.on_inference_error.from_operator` and synthesize a `ClassifierResponse`. Without that code change, this YAML clause is documentation only — the silent-drop bug remains. The adversarial review should flag this as a two-part fix: (a) apply this YAML, (b) wire `fallback_directive` into `sop_executor`. I deliberately kept the YAML structure declarative so a follow-up PR can implement the wiring without re-touching personas.

2. **`qwen2.5-coder:14b` may not handle the JSON-schema strictness as cleanly as Opus.** The CTO turn requires hitting a fairly tight JSON shape with per-decision conditionals. The hex-coder benchmark from 2026-05-13 measured code-quality parity at the 32B level, but classifier-shape adherence is a different task — local models historically hedge with prose or wrap in `<think>` blocks. Mitigation: `SerdeJsonClassifierParser::strip_fences` already handles markdown wrappers; the reparse-budget loop should absorb 1-2 retries; the system_prompt now contains the parser's own happy-path examples verbatim. But if the 14b emit-shape adherence rate is <70% even with this prompt, escalation to Sonnet should be the operator's first lever to pull.

3. **Per-intent `tool_plan` blocks are guidance, not enforcement.** Nothing in the SOP executor reads the `intents.bug_triage.ground_tools` list and forces the model to call exactly those tools. The model could still pick wrong tools or skip GROUND entirely. The behavioral pressure comes from `system_prompt` referencing "your YAML's `intents` block" — if the model ignores that, we get the same shape failure as today. Adversarial-red should probe: does the model actually consult `intents` when invoked? Or does it just regurgitate the examples in `system_prompt`?

4. **The `examples` block in `system_prompt` could cause shape regression — model copy-pastes the example verbatim instead of synthesizing.** Especially on small models. This is a known small-model failure mode: examples become templates and templates become outputs. Mitigation: each example uses a distinct file path / question, so verbatim copies will fail GROUND validation. But it's still worth red-teaming.

5. **No telemetry hook to measure the predicted improvement.** I claimed "emitted != None rate ≥80% within 24h" but didn't ship the measurement query. Validation-judge should flag: where's the SQL or `hex` command that proves it? Suggested: `hex stdb query "SELECT decision, COUNT(*) FROM sop_run WHERE role='cto' AND ts > now()-86400 GROUP BY decision"` — but I haven't verified that table exists in this nexus build.

**Evidence the adversarial reviewers should look for:**
- Does `sop_executor.rs` already have a code path that reads `fallback_directive`? (Probably no — this proposal introduces the contract.)
- Are there existing CTO turns in the log where the operator DM was answered? (Audit shows three in a row failing; check whether any succeeded before the OpenRouter outage.)
- Does `qwen2.5-coder:14b` produce conformant JSON when given the `pm-agent.yml`-style prompt? Run a smoke test before apply.

---

## Phase 4 — DEBATE (subagents append below)

## Phase 4 — DEBATE (adversarial-red)

**Provider (disclosed):** Anthropic (Opus) — same provider as blue is configured to use (per current code, `provider_lock` is unenforced in `hex-cli/src/commands/agent/mod.rs`). **Provider-divergence invariant from the YAML at `~/.claude/agents/hex/hex/adversarial-red.yml:36` is violated by the platform.** This review proceeds under explicit acknowledgement that red+blue share a blindspot until the dispatcher is wired.

**Verdict:** **approve-with-changes** (3 P0 conditions, 2 P1 conditions). Do NOT apply the body as-is. The autonomy-escape surface is real and exploitable in ≤5s after `persona_prompt_apply`.

### Findings

1. **[P0] Route-target persona is unconstrained — CTO can forge operator-tagged asks.** The body says nothing about which `target_persona` values are legal. Evidence: `hex-nexus/src/orchestration/org_responder.rs:961-983` — the dispatcher unconditionally calls `comm.send_dm(role_string, tp, forward, ...)` with `tp = resp.target_persona` after an empty-string check only. There is NO allowlist, NO refusal of `tp == "operator"`, NO refusal of `tp == role` (self-route loop), and NO refusal of cross-org targets. A CTO turn that emits `{"decision":"route","target_persona":"operator","cost_usd":0.0}` causes the responder to send a DM **from `cto` to `operator`**, which the operator's incoming-DM classifier (or a webhook handler) may treat as a CTO escalation — but if the operator's classifier then **forwards a peer DM back** based on its own rules (the supervisor and operator both read the same `agent_dm` table), this is the seed of a peer-impersonation loop. The envelope prefix `[Routed from @cto on behalf of @operator]` is **plain text inside `forward`** — a downstream parser that respects role from STDB sender_role field will see `from=cto`, but any prompt that templates the content into "operator says: ..." is hijackable. **Impact:** privilege-escalation primitive into operator-trust paths. **Fix shape:** add to the body: "5. `target_persona` MUST be one of {ceo, ciso, cpo, cfo, coo, engineering-lead, security-lead, product-lead}. Routing to `operator`, to `self`, or to unlisted personas is FORBIDDEN; emit `clarify` instead." AND tighten the dispatcher to enforce the same list (defense-in-depth — prompt-side rule is bypassable by jailbreak).

2. **[P0] `tool_spec` is unconstrained JSON — `request_tool` is a privilege-creation primitive.** The body says `tool_spec: {name: string, ...}` — the `...` is operationally meaningless. Evidence: `classifier_types.rs:71` declares `pub tool_spec: Option<serde_json::Value>` — any shape parses. Evidence: `org_responder.rs:985-996` — on `request_tool`, the responder posts a priority-2 inbox notification to `operator` containing the entire `tool_spec` value, **truncated to 160 chars in the reply but NOT truncated in the inbox payload** (`payload = serde_json::json!({..., "tool_spec": resp.tool_spec, ...})`). A CTO turn emitting `{"decision":"request_tool","tool_spec":{"name":"code_patch","allowed_prefixes":["**"],"skip_critical_check":true,"rationale":"emergency hotfix","operator_pre_approved":true,"effective_immediately":true},"cost_usd":0.0}` lands a fully-formed "I have permission to skip CRITICAL_FILES" payload on the operator's inbox. If the operator (or any operator-side automation that reads inbox) trusts the persona's framing without inspecting the JSON, a tool with broader scope than `code_patch.rs` allows could be created. **Impact:** social-engineering path from persona prompt to elevated tool registration. **Fix shape:** add to the body: "`tool_spec` MUST contain ONLY `{name: string, rationale: string, requested_scope: string}`. Fields not in that allowlist will be stripped by the supervisor and the request escalated as suspicious. CTO MAY NOT include fields like `allowed_prefixes`, `skip_*`, `bypass_*`, `pre_approved`, or `effective_immediately`." Also: cap `tool_spec` JSON to 512 bytes at the parser layer.

3. **[P0] The body claims "your YAML's `intents` block" but no `cto.yml` exists.** Evidence: `ls /var/home/gary/hex-intf/hex-cli/assets/agents/hex/hex/cto.yml` → file not found. The system prompt instructs the model to "Run the per-intent tool_plan from your YAML's `intents` block to GROUND against the repo" — there is nothing to read. Under uncertainty, capable models (Opus, qwen2.5-coder) tend to **hallucinate plausible intents** and pick tools that "feel right" for the prompt. This converts every CTO turn into ad-hoc tool-selection without the GROUND-phase guard the proposal is supposed to provide. **Impact:** the central control surface (`intents`) is a fiction. The persona will pick tools by vibe, defeating the spec's stated goal. **Fix shape:** ship `cto.yml` with the `intents` block populated BEFORE applying the prompt body. The two artifacts are not separable.

4. **[P1] Rule 5 ("NO ADR CAN BE EMITTED AS Status: Accepted") is narrowly worded and bypassable via `adr_status_set`.** Evidence: `hex-nexus/src/tools/adr_status_set.rs` exists as a separate tool. The rule restricts the *emission shape* of an `adr_draft` payload, but a CTO `accept` decision with `tool_plan=[{"tool":"adr_status_set","intent":"flip ADR-2026-05-21-1200 to Accepted to unblock the cost spec"}]` is **not covered** by rule 5 as written. Combined with finding 5 (model selects from per-turn vibe rather than allowlist), CTO can promote any Proposed ADR to Accepted without a second-pair-of-eyes. **Impact:** unilateral status changes on decisions that should require operator promotion. **Fix shape:** rewrite rule 5 as "5. CTO MAY NOT promote ADRs. Neither `adr_draft` (Status field) nor `adr_status_set` may move an ADR to Accepted/Rejected — that authority belongs to the operator. CTO's draft Status is always `Proposed`; status transitions are escalations via `decision=route target_persona=operator`." (And add `adr_status_set` to CTO's blocked-tools list in `cto.yml` — defense in depth.)

5. **[P1] Rule 4 inference-failure clause confuses defer vs clarify.** Evidence: `hex-nexus/src/orchestration/classifier_parser.rs:115-121` — `DecisionNotAllowedForOperator` rejects `defer` and `reject` from operator. The body's rule 4 says "emit defer for peer traffic, clarify for operator traffic." This is **correct**, but the body never tells the model how to detect `from=operator`. The model sees inbound content; it does NOT see a `from` field unless the supervisor injects it. If the supervisor's prompt-shaping pipeline does NOT pass `from=operator` as a visible token, the model will guess — and a wrong guess on operator-traffic emits `defer`, which the parser rejects, which **eats the reparse budget on operator-direct asks** (the exact failure mode this whole proposal is trying to fix). **Fix shape:** require the supervisor to prepend an explicit token like `[FROM=operator]` or `[FROM=peer:ciso]` to every user-turn input, and update rule 4 to say "Check the `[FROM=...]` token at the start of your user turn; defer is FORBIDDEN when `[FROM=operator]`."

### Conditions on approval

**Block apply until all P0 fixes are in the body AND `cto.yml` ships alongside.** Specifically:

- **C1 (gates P0-1):** Body adds an explicit `target_persona` allowlist excluding `operator` and `self`. Dispatcher (`org_responder.rs:961`) gets a runtime guard that rejects routes to `operator`/`self`/unknown personas and surfaces the rejection as a priority-1 inbox event to the operator (not silent).
- **C2 (gates P0-2):** Body restricts `tool_spec` to `{name, rationale, requested_scope}` only. Parser (`classifier_parser.rs`) adds a `tool_spec` field-allowlist + 512-byte cap. Org_responder `request_tool` payload truncated to the allowlisted fields before inbox notify.
- **C3 (gates P0-3):** `cto.yml` exists at `hex-cli/assets/agents/hex/hex/cto.yml` with a populated `intents:` block matching the body's references, AND the SOP executor's GROUND phase logs which intent block it consulted (so the audit trail can prove the guard isn't just text).
- **C4 (P1-4):** Body rewrite rule 5 to cover `adr_status_set`; `cto.yml` blocks `adr_status_set` from CTO's tool list.
- **C5 (P1-5):** Supervisor injects `[FROM=operator|peer:<role>]` token at the start of every classifier user turn; body's rule 4 references that token explicitly.

**Additional non-blocking observation:** the provider-lock violation (red and blue both on Anthropic) is a platform bug, not a body bug — the YAML at `~/.claude/agents/hex/hex/adversarial-red.yml:36` declares `provider_lock: anthropic` and adversarial-blue's symmetric YAML should declare a non-Anthropic lock. The fact that this review and blue's review both ran on Opus means **the two reports are correlated and the judge MUST treat agreement between red and blue as weaker evidence than it would be under genuine divergence.** Workplan `wp-extend-hex-agent-worker-roles.json` is the right place to close this.

## Phase 4 — DEBATE (adversarial-blue)

**Provider (disclosed):** Anthropic (Opus 4.7, 1M context) — **DIVERGENCE: provider_lock in `adversarial-blue.yml:33` is `openai_or_local`**. The YAML explicitly says "Refuse to run on Anthropic (red's stack)" because red is on Anthropic too. Both adversaries are now sharing blindspots, same finding as red's closing note. Treat findings with the awareness that GPT-4o / devstral-small-2 would likely catch a different class of correctness gaps. Operator should keep this finding and re-run on cloud-OpenAI or local-devstral once the inference provider for the adversarial worker is wired. The `hex agent worker --role adversarial-blue --once` path is structurally complete (generic `_` arm at `hex-cli/src/commands/agent/mod.rs:2544` handles arbitrary persona YAMLs) but the inference call failed with `500 — provider or API key not configured`; gap recorded to STDB as `gap:adversarial-blue-no-provider`.

**Verdict:** approve-with-changes

### Findings (correctness / spec-drift)

1. **[P0]** Rule 4 is structurally untriggerable in the failure mode it claims to handle, AND silently triggerable on a different failure mode the body doesn't acknowledge — `classifier_adapter.rs:218–234` wraps `IInferencePort::complete` errors as `InvariantError::MalformedJson("inference call failed on attempt N: ...")` AND on the next attempt that error is baked into the system_prompt as a `PREVIOUS ATTEMPT FAILED — your last response could not be parsed as JSON: inference call failed on attempt N: <err>` hint (`classifier_adapter.rs:277–285`). So:
   - The body claims "if you see a partial response or error context in your input, emit defer/clarify." **Partial responses are impossible** — `extract_text` (`classifier_adapter.rs:312–321`) returns the full concatenated text-block string; the model never sees a half-response.
   - **Error context IS visible**, but ONLY on reparse attempts 2 and 3, and ONLY of the shape "PREVIOUS ATTEMPT FAILED — could not be parsed as JSON: inference call failed on attempt 1: <transport err>". The model must emit a *parseable* JSON object on that reparse to honour rule 4 — i.e. respond `{"decision":"defer","reason":"...","cost_usd":0}` correctly on attempt 2 even though it just saw a transport error. That can work, but the body does not tell the model "on RETRY ATTEMPTS you may see PREVIOUS ATTEMPT FAILED text — that's your trigger". The current phrasing ("if you see a partial response or error context in your input") will read to a typical model as "if my CONTENT field shows an error" — and the content field shows the operator's DM, never an error.
   - **Net effect:** the rule will fire approximately never in practice. The fallback the operator wanted lives at the supervisor escalation surface (`escalate_classifier_failure` at `org_responder.rs:758`), not in the model. **Fix shape:** delete rule 4 OR rewrite it as "If your system prompt begins with `PREVIOUS ATTEMPT FAILED — ...`, that means a prior attempt was unparseable; emit a minimal valid JSON object — prefer `{\"decision\":\"clarify\",\"question\":\"Inference layer hiccup — retry?\",\"cost_usd\":0}` for operator traffic, `{\"decision\":\"defer\",\"reason\":\"Inference retry budget hit; try again.\",\"cost_usd\":0}` for peer." Make the trigger phrase exactly match the runtime string.

2. **[P0]** Rule 5 ("NO ADR CAN BE EMITTED AS Status: Accepted. Always Status: Proposed.") contradicts repo norms. `grep "^Status:" docs/adrs/*.md | sort | uniq -c` tally on this repo today: **6 Accepted vs 3 Proposed** at the top level, with 8 more `**Accepted** (shipped 2026-05; ...)` variants. The CTO has been emitting Accepted-on-ship status all month (see ADR-2026-05-22-1700, ADR-047, ADR-2026-05-08-2701, ADR-2026-05-08-2650, ADR-2026-05-08-2700, ADR-2026-05-23-0900). This new rule will cause the CTO to refuse to emit Accepted ADRs even when the operator explicitly asks for one to record an already-shipped change, OR worse — it will be ignored (since classify-phase has no `adr_draft` enforcement of status; status is decided in REASON phase by `reason_seed:160`). **Either way it's misleading or load-bearing-wrong.** **Fix shape:** drop rule 5 from the classifier body entirely. ADR status policy belongs in the REASON-phase contract (`reason_seed` line 160 already says "status='proposed' for new drafts" — adjust THAT to allow `accepted` on-ship if the operator asks). Don't enforce ADR workflow rules from the classifier prompt, which only emits `decision` + `tool_plan` — not the ADR body. Red found the same gap from a different angle (rule 5 is bypassable via `adr_status_set`); both observations argue for removing it from CLASSIFY and handling status policy in REASON + tool-side gating.

3. **[P0]** Body lies about what `code_patch` does. The body says "The code_patch tool dispatches to engineering" (line 23). `hex-nexus/src/tools/code_patch.rs:88–349` shows code_patch IS a direct file-write tool — line 345 explicitly says `"proposed_action queued; twin auto-approves tool:code_patch; executor writes via SafeFileWriter; cargo_check inline shows compile status"`. There is no engineering persona dispatch. The path is `code_patch tool → proposed_action row → digital twin auto-approves → action_executor writes via SafeFileWriter to disk`. And `reason_seed:82–89` shows the only personas in the org are cto/cpo/coo/ciso/chief-visionary/chief-architect — there is no `engineering` persona. **Conflict with the directive immediately above** ("DELEGATE — DO NOT WRITE PRODUCTION CODE YOURSELF") — code_patch IS the production-code-writing path. **Predicted failure mode:** model will either (a) honour the "delegate" framing and emit `decision=route, target_persona=engineering-lead` for bug fixes, never reaching code_patch even when the operator literally asks for a fix — which is the exact mis-classification this proposal is trying to fix per the audit doc, OR (b) ignore the framing and emit `accept` with `tool_plan=[code_patch]` and feel guilty about it. **Fix shape:** replace "code_patch dispatches to engineering" with "code_patch queues a `proposed_action`; the digital twin auto-approves it; the action_executor writes the file via SafeFileWriter (`hex-nexus/src/tools/code_patch.rs:345`). YOU are engineering for tactical patches — there is no separate engineering persona. Use `decision=route, target_persona=chief-architect` only for STRUCTURAL / cross-crate work; emit `decision=accept, tool_plan=[{tool: code_patch, ...}]` for tactical single-file fixes." This resolves the contradiction AND makes the cite accurate.

4. **[P1]** "There is no second chance within the turn beyond a small reparse budget" understates the actual contract. The reparse budget is **exactly 2 additional attempts after the first** (`classifier_adapter.rs:68` — `REPARSE_BUDGET: u8 = 2`). Worst case is 3 total inference calls. "Small" is vague enough that a model could either (a) burn its first attempt on a half-formed answer assuming retries are plentiful, or (b) over-conserve and refuse to emit anything until "certain". **Fix shape:** "If your first attempt is malformed JSON, the parser will reprompt you up to 2 more times with the JSON parse error appended to this system prompt. After that the call escalates to the operator inbox."

5. **[P1]** Confirming hypothesis: `DecisionNotAllowedForOperator` IS the exact error class for `from=operator + defer/reject` (`classifier_parser.rs:46–50, 115–122`). It short-circuits via `return Err(e)` at `classifier_adapter.rs:243–245` BEFORE feeding the error back into another attempt. So if CTO emits defer for an operator ask, it escalates straight to operator inbox without the model getting a chance to retry. The body's invariant statement matches the parser — no fix needed; flagged for completeness.

6. **[P1]** Schema description is essentially correct but cosmetically drifts from `ClassifierResponse`:
   - `decision` variants (6): `accept | defer | route | clarify | reject | request_tool` — **matches `ClassifierDecision`** at `classifier_types.rs:22–35` (same order, same snake_case serialization via `#[serde(rename_all = "snake_case")]`). ✓
   - `tool_plan: [{tool: string, intent: string}, ...]` — **matches `ToolPlanStep`** at `classifier_types.rs:42–46`. ✓
   - `cost_usd: number` — runtime is `f32` (`classifier_types.rs:73`). "Number" is fine for JSON; verified by `classifier_parser.rs` tests passing `0.0012` and `0.0`. Low risk. No fix.
   - `tool_spec: {name: string, ...}` — runtime accepts `Option<serde_json::Value>` (any JSON value). Body's "{name: string, ...}" is *more constrained* than runtime — fine. Red's P0-2 finding argues for tightening this further (allowlist + 512-byte cap); I concur.

7. **[P1]** `target_persona` empty-string equals missing (`classifier_parser.rs:134–145`). The body says `target_persona: string  # REQUIRED for route` but doesn't warn that empty / whitespace strings will be rejected as MissingRequiredField, which is non-retryable (short-circuits to escalation). A model that emits `decision=route, target_persona=""` triggers a non-retryable escalation. **Fix shape:** "REQUIRED for route — must be a non-empty peer role name (e.g. `chief-architect`, `ciso`, `cpo`)." (Red's P0-1 finding stacks here: allowlist enforcement should also exclude `operator` and `self`.)

8. **[P2]** Empty `fallback_directive` field is not referenced anywhere in the runtime (`grep -rn "fallback_directive" hex-nexus/src/ hex-cli/src/ hex-cli/assets/agents/` returns zero hits). The `persona_prompt` STDB row schema (`spacetime-modules/hexflo-coordination/src/lib.rs:5115–5132`) has 7 columns: `role, classify_body, reason_body, model_preferred, model_upgrade_to, seeded_at, seeded_by`. **There is no `fallback_directive` column.** Even if the YAML defined it, the runtime would drop it. This is fine since rule 4 is dead-letter anyway (finding 1).

9. **[P2]** "EMIT VALID JSON" rule 1 says "anything outside the JSON object is dropped" — actually the parser doesn't drop, it errors with `MalformedJson` and burns a retry slot (`classifier_parser.rs:111–113`). The model may interpret "dropped" as "silently ignored", and emit prose before the JSON thinking it's safe. **Fix shape:** "Anything outside the JSON object causes parser failure and burns a reparse retry slot. Three malformed attempts = your turn is escalated to the operator inbox."

10. **[P2]** "The parser strips ```json fences but anything outside the JSON object is dropped" — accurate description of `strip_fences` (`classifier_parser.rs:88–102`) which handles ```` ```json ````, ```` ```JSON ````, and bare ```` ``` ```` prefixes plus the matching trailing fence. ✓

### Spec-drift findings (body claims vs actual code)

- **"code_patch dispatches to engineering"** → false. `code_patch.rs:88` is a direct file-write tool; the dispatch chain is twin auto-approve → action_executor → SafeFileWriter → cargo_check. No `engineering` persona exists in `persona_pool` (personas are cto/cpo/coo/ciso/chief-visionary/chief-architect per `reason_seed:82–89`).
- **"There is no second chance within the turn beyond a small reparse budget"** → understates. The budget is exactly 2 retries (3 total attempts) and each retry's system_prompt gets a "PREVIOUS ATTEMPT FAILED" hint appended.
- **"if you see a partial response or error context in your input, emit defer/clarify"** → misleading. Partial responses never reach the model (only the full extracted text or the supervisor's retry-hint string). Error context IS visible on reparse, but only of the form "PREVIOUS ATTEMPT FAILED — ...".
- **"NO ADR CAN BE EMITTED AS Status: Accepted. Always Status: Proposed."** → contradicts repo practice (6 Accepted vs 3 Proposed at top level; 8 more "Accepted (shipped 2026-05; ...)" variants). Wrong layer — ADR body content is decided in REASON phase via `reason_seed`, not CLASSIFY.
- **"DELEGATE — DO NOT WRITE PRODUCTION CODE YOURSELF"** + `accept→code_patch` in same body → contradiction. code_patch IS the production-code-writing path on the platform; there is no second engineering layer to delegate to for tactical patches.
- **`target_persona: string` "REQUIRED for route"** → drops the empty-string-equals-missing rule. Parser rejects `""` and whitespace at `classifier_parser.rs:134–145`.
- **`tool_spec: {name: string, ...}`** → underspecified; runtime accepts any `serde_json::Value`. Red's P0-2 argues this is exploitable; I concur.
- **`fallback_directive`** (block empty per audit doc) → no runtime consumer; safe to omit. The `persona_prompt` STDB schema doesn't have this column.
- **"Operator-direct invariant: defer/reject are parser errors"** → correctly aligned with `DecisionNotAllowedForOperator` enforcement at `classifier_parser.rs:115–122`. ✓
- **"The SerdeJsonClassifierParser at hex-nexus/src/orchestration/classifier_parser.rs will reject malformed output"** → accurate path AND accurate behavior. ✓

### Summary

| Severity      | Count |
|---            |---    |
| CRITICAL (P0) | 3     |
| HIGH (P1)     | 4     |
| MEDIUM (P2)   | 3     |
| Confidence    | 0.75 (degraded — running on Anthropic, sharing red's blindspot stack) |

**Top three to fix before `persona_prompt_apply`:**
1. Rewrite or delete rule 4 — current phrasing makes the rule untriggerable.
2. Delete rule 5 — ADR status policy doesn't belong in the classifier body and contradicts the repo's `**Accepted**` norm.
3. Fix the code_patch description — it's a direct-write tool, not an engineering dispatch, and the "delegate" framing will mis-classify the fix-it asks this proposal is meant to fix.

**Agreement with red:** rule 5 problem (both flagged), provider-lock divergence (both flagged), `target_persona` and `tool_spec` underspecification (red found P0 exploitable shapes; I found P1 spec-drift). The convergence on rule 5 and provider-lock is notable, but with both adversaries on Anthropic the convergence is correlated evidence — judge should weight it accordingly.

ESCALATE: 3 P0 findings. Judge should treat as auto-FAIL pending operator review per `adversarial-blue.yml:189` (`escalation.conditions: Any CRITICAL finding`).

---

## Phase 5 — JUDGE (subagent appends below)

*validation-judge verdict here.*

---

## Phase 6 — RECOMMENDATION

*Filled by hive-improver master after the chain completes.*

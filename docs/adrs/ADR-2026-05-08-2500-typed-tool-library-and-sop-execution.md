# ADR-2026-05-08-2500 — Typed Tool Library + SOP Execution (the Missing Spine)

Status: **Accepted**
Date: 2026-05-09
Supersedes / refines: ADR-2026-05-08-2400 (personas as commitment-creators), ADR-2026-03-24-0130 (declarative swarm behavior)
Related: paradigm-debate-judge-verdict, ADR-2026-05-08-2300 (digital twin), ADR-2026-05-08-2200 (resource supervisor), ADR-2026-05-08-1126 (merge gate)

## Context

Tonight's diagnosis (verified by ADR audit — `0 / 206` ADRs mention "tool library"):

Hex has shipped **massive substrate**: SpacetimeDB integration (110 ADRs touch it), inference routing across 4 tiers (102 ADRs), swarm/hexflo coordination (119 ADRs), tree-sitter analyzer (28 ADRs), RL leader selection (28 ADRs), supervisors (23 ADRs), worktree management (42 ADRs), hexagonal architecture enforcement (51 ADRs), dashboard (86 ADRs), merge gate (12 ADRs), secret grants (13 ADRs), neural lab (9 ADRs), messaging (11 ADRs).

Hex has shipped **zero typed tool primitives the LLM can compose deterministically**. Every persona's `tools:` block is empty. The only action a persona can take is `write_chat_reply`. This is why every persona LARPs — there is literally nothing else they can do.

The result is a Unix system with kernel, scheduler, file system, network stack, and a beautiful shell — but no programs. The agents have nothing to call.

This ADR closes the gap. The investment in substrate was not wasted — it is exactly the foundation the tool library will be built on. The work is wrapper code, schema design, and one state machine — not new substrate.

## Decision

Add two layers, both new, both load-bearing:

### Layer 1: Typed tool library

A Rust module `hex-nexus/src/tools/` exposing typed primitives that wrap existing capabilities. Each tool:

- has a typed Rust function signature (input struct, output struct)
- has a JSON-schema export for Anthropic function-calling
- is deterministic (caches identical inputs)
- is registered in a single `ToolRegistry` consulted by the inference path

Tonight's first wave (4 tools sufficient for the end-to-end demo):

| Tool | Wraps | Signature |
|---|---|---|
| `cargo_check` | `cargo check` subprocess | `(crate: String) → {ok: bool, errors: Vec<CargoError>}` |
| `repo_grep` | ripgrep subprocess | `(pattern: String, glob: Option<String>) → Vec<Match>` |
| `adr_draft` | writes file via existing SafeFileWriter path | `(id, title, status, body) → ProposedAction` |
| `escalate_to_operator` | inserts `inbox_notification` row | `(reason, urgency, options) → InboxId` |

Second wave (after demo proves the pattern, queued for tomorrow):
`repo_read`, `git_log`, `cargo_test`, `cargo_clippy`, `analyze_deps` (wraps hex-analyzer), `adr_search` (wraps `hex adr search`), `workplan_emit`, `boundary_check` (wraps hexagonal analyzer), `merge_request_open`, `swarm_init`.

### Layer 2: SOP executor

Replace `org_responder::process_role`'s single-LLM-call hot path with a 5-phase state machine per inbound persona DM:

```
PHASE 1  CLASSIFY        zero LLM cost; regex + fast STDB lookup
                         output: intent ∈ {adr_draft, arch_review, code_question,
                                           bug_triage, roadmap, paradigm_question}
                         GATE: paradigm_question → escalate + halt
                         GATE: outside-domain → handoff + halt

PHASE 2  GROUND          deterministic tool calls only; NO LLM
                         runs in parallel: repo_grep, cargo_check, adr_search
                         output: ground_pack (typed findings, ~2-4 KB)

PHASE 3  REASON          ONE frontier LLM call with function-calling enabled
                         input: operator message + ground_pack + tool registry
                         LLM may call tools (repo_grep, cargo_check, etc.)
                         LLM MUST emit one structured action via tool call:
                           adr_draft | workplan_emit | escalate_to_operator | acknowledge
                         GATE: schema validation; off-schema → 1 retry → escalate

PHASE 4  VERIFY          deterministic; the oracle
                         per output kind: schema_validate, cargo_check, file_exists
                         GATE: fail → loop to PHASE 3 with concrete error (max 3) → escalate

PHASE 5  EMIT            STDB write (proposed_action) + structured chat card
                         no free prose; chat renders typed fields
```

Tonight's first persona to execute the SOP: **CTO**. Other personas are wired to the same SOP shape with role-specific tool subsets in a follow-on workplan.

### What this preserves

- All existing infrastructure (STDB, inference routing, supervisors, merge gate, dashboard, etc.) is the substrate the tools wrap. Nothing thrown away.
- Persona addressing (`@cto`, `@cpo`, …) — still the operator's natural delegation primitive. Routes into a typed pipeline now.
- Digital twin, drafter, executor — still consume `proposed_action` rows from the factory. Tool library produces those rows directly now (skipping the drafter for tools that emit artifacts), or via the existing drafter for narrative artifacts.
- ADR-2026-05-08-2400 strict Confirm/Silent contract — replaced by tool calls. The Confirm: sentinel is now an output of the `acknowledge` tool. Silent is a no-op.

### What this kills

- The post-hoc `commitment_parser` regex. With function-calling, the LLM emits structured output; no parsing needed.
- The free-form prose path in `org_responder::generate_reply`. Replies are tool-call results, period.
- The "twin LLM reviews drafter LLM" recursion. Phase 4 verifier is `cargo_check` / schema validate, not another inference.
- The board-mode prompt addendum. With CLASSIFY + atomic claim, board threads route to the right single persona in their domain. No multi-persona chat.

## Consequences

Positive:
- Persona drift to off-topic content becomes structurally impossible — the LLM's only output channel is a typed tool call.
- Inference cost drops further: Phase 2 ground_pack often answers the question without LLM (operator asks "what files touched X" → repo_grep alone suffices).
- Frontier model used only for Phase 3 reasoning; Phases 1/2/4/5 are free.
- Substrate investment pays off: every existing capability becomes a tool the agents can use.
- End-to-end verification: an artifact's existence + cargo_check pass is the proof, not a twin's opinion.

Negative:
- Phase 3 REASON requires Anthropic-tier function-calling (Sonnet 4 or Opus). Local Ollama models that don't support function-calling cannot be Phase 3 personas. Tier router enforces this.
- 4-tool bootstrap is minimum; the persona's domain coverage grows linearly with tool library breadth. Initial demo will look thin until second-wave tools land.
- The `phases:` blocks in persona YAMLs that were aspirational become mandatory contracts; mismatch with the runtime SOP is a warning at startup.

## Validation

End-of-night smoke (the only thing that matters tonight):

1. Operator sends: `@cto draft an ADR for the typed tool library`
2. CTO Phase 1 classifies as `adr_draft`
3. CTO Phase 2 grounds: `repo_grep("typed.tool|tool.library")`, `repo_grep("function.call|anthropic.tool")`, `adr_search("ADR-2026-05-08-2500")`
4. CTO Phase 3 reasons (Sonnet 4): emits `adr_draft(id="2605082501", title="...", status="proposed", body="<3-page ADR>")`
5. CTO Phase 4 verifies: `adr_schema_validate(body)` passes
6. CTO Phase 5 emits: `proposed_action(kind=file_write, payload={path, content})` row written, structured chat card sent to operator
7. Existing executor path picks up the proposed_action and writes `docs/adrs/ADR-2026-05-08-2501-*.md`
8. Operator reads the file in the morning. Content **directly answers** the typed-tool-library ask. NOT generic enterprise tooling LARP.

If step 8 produces an on-topic ADR, the architecture works and we expand. If not, the architecture fails at the schema boundary — debuggable in one place rather than scattered across personas.

## Out of scope (queued for follow-on workplans, in priority order)

1. Second-wave tools (8 more wrappers)
2. Per-role tool subsets (CPO, CISO, COO, leads — each with role-flavored tool slices)
3. Dashboard view: phase trace per persona turn (`#/persona-trace`)
4. Tool result caching layer
5. `acknowledge_no_action` flow + operator dashboard widget
6. Migration of remaining personas from Confirm/Silent contract to SOP execution

## Migration path from tonight's state

- ADR-2026-05-08-2400 (personas as commitment-creators) is **superseded for the CTO path** by this ADR. CPO/COO/CISO/chief-visionary remain on the Confirm/Silent contract until their tools ship in Wave 2.
- `org_responder::process_role` gains a feature flag `HEX_SOP_PERSONAS` (CSV). When the role is in the flag, route through SOP. Otherwise, current behavior. Tonight: `HEX_SOP_PERSONAS=cto`.
- Existing `commitment` table continues to receive entries from non-SOP personas; SOP personas write directly to `proposed_action` (skipping commitment).
- Digital-twin loop continues unchanged for SOP-emitted proposed_actions; the twin verifies the action against operator memory as a SECOND opinion (defense in depth), but the SOP's own Phase 4 verifier is the primary gate.

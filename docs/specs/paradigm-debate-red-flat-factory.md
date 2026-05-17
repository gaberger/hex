# Paradigm Debate — Red Position: Flat Factory Pipeline

**Adversary:** adversarial-red
**Date:** 2026-05-08
**Topic:** Should hex's organizational model be a hierarchical executive org chart (CTO/CPO/COO/CISO/chief-visionary + leads) or a flat factory pipeline (drafter → twin → executor → judge → audit)?
**Position to defend:** **Flat factory wins. Demote the org chart to LARP.**

---

## 1. Position statement

hex is a code factory. Code goes in as intent, validated artifacts come out
as commits. Every successful artifact today travels a flat, named, gated
pipeline: `commitment_open → drafter → proposed_action → digital-twin →
action_executor → SafeFileWriter → executed_action audit`
(ADR-2605082300). Every successful merge today travels the flat
red/blue/judge merge gate (ADR-2605081126). The hierarchical exec layer
sits *next to* both pipelines, holds zero file-write tools, emits zero
artifacts, and consumes inference budget producing chat-shaped filler.
hex should formalise the pipeline that already does the work and retire
the org chart that doesn't.

---

## 2. Evidence from this session (2026-05-08) — hierarchical failure modes

These are observed behaviours from today, not hypotheticals.

**E1 — Personas have no tools.** Inspect `hex-cli/assets/agents/hex/hex/cto.yml`,
`cpo.yml`, `coo.yml`, `ciso.yml`. The phases are
`assess / plan / coordinate / review / report`. The
delegates_to list names other personas (`engineering-lead`,
`backend-lead`, etc.). There is no `tools:` block. Compare to
`hex-coder.yml` or `adversarial-red.yml`, both of which declare
`tools.required: [Read, Glob, Grep, Bash]`. The execs literally cannot
write a file, run `cargo check`, or open a PR. Every "I'll coordinate
with engineering-lead" is a chat string with no downstream effect.

**E2 — Empty filler is the default output.** Per the brief: personas reply
with "I'll facilitate coordination" / "I'll communicate with the
engineering lead" unless heavily prompt-engineered. The grounding
adapter `hex-nexus/src/orchestration/repo_grounding.rs` and the
anti-fabrication memory note `feedback_no_persona_fabrication` exist
**because** the default exec output is fabricated theatre. The flat
factory has no equivalent failure mode: a drafter that emits empty
content produces an empty `proposed_action`, which the twin rejects in
one inference call. The hierarchy's failure is invisible and rate-limited
only by operator patience.

**E3 — Five PLANs, zero confirmations.** Per the brief: with the structured
PLAN/Confirm/Silent protocol, all 5 execs wrote PLANs simultaneously
because of STDB `read_by` lag. A hierarchy without atomic claims is just
five parallel monologues. The flat factory's `commitment_open →
drafter` transition is a single STDB row update on a single key — there
is no "five drafters race" because the queue is the coordinator. (Same
pattern proven by `worker_pool_intent` / `worker_process` in the
supervisor — see memory `feedback_supervisor_in_stdb`.)

**E4 — Drift to off-topic.** Per the brief: asked about persona tooling
gaps, the CTO drafted "enterprise CI/CD." This is what happens when the
agent's identity is "be a CTO" rather than "produce a `proposed_action`
of `kind=file_write` with `payload.path` and `payload.content`". A
drafter pinned to a typed output schema cannot drift into management
prose; it either fills the schema or fails validation.

**E5 — Flat coordination is already the proven path.** ADR-2605081126's
merge gate is `validation-judge + adversarial-red + adversarial-blue`
voting in parallel, 2-of-3 + judge-pass to merge. No CTO. No engineering
lead. Three named functional roles, deterministic transitions, auditable
ballot. It is the mechanism that recovered the trunk from the 2026-05-07
hijacker incident. Meanwhile the exec layer has zero recorded incident
recoveries.

**E6 — The operator memory IS the standards manual.** ADR-2605082300
Context paragraph: "*the operator already has a documented
decision-making style in `~/.claude/.../memory/*.md` (15+ rules at last
count)*… that memory IS the operator's standards manual — it's the
right authority for an automated stand-in." The digital-twin loads
operator memory as system prompt. The execs cannot, by construction,
out-vote a memory file the operator wrote themselves. Hierarchy where
the leaves outrank the root is theatre.

---

## 3. The flat factory — concrete design

### Stages (each is a STDB table + reducer)

| # | Stage | Worker role | STDB row produced | Success criterion |
|---|---|---|---|---|
| 1 | **intake** | `intake-classifier` | `intent_open(text, classification)` | classification ∈ {chat, code-change, ops, infra} |
| 2 | **spec** | `behavioral-spec-writer` (exists) | `spec_open(path, behaviors[])` | spec parses + ≥1 behavior |
| 3 | **draft** | `drafter` (exists, `orchestration/drafter.rs`) | `proposed_action(kind, payload, proposed_by)` | payload validates against `kind` schema |
| 4 | **review** | `digital-twin` + `adversarial-red` + `adversarial-blue` (parallel) | `verdict(approve|reject|escalate)` × N | 2-of-3 approve AND twin not reject |
| 5 | **execute** | `action_executor` (exists) | `executed_action(evidence_path, sha)` | SafeFileWriter succeeds; cargo-check passes |
| 6 | **audit** | `auditor` | `audit_row(stage_id, latency, cost, outcome)` | row persisted; dashboard tile updates |

### Queues (each transition is an STDB query)

```
intent_open WHERE classification='code-change' AND claimed_by IS NULL
        → spec_open (claim by spec-writer)
spec_open WHERE behaviors[] != [] AND drafted=false
        → proposed_action (claim by drafter)
proposed_action WHERE verdicts.count >= 3
        → executed_action OR commitment_abandon
```

No agent reads from the queue it writes to. No cross-stage shortcut. A
stage that fails N times trips a circuit breaker (sibling of `persona_health`
ban-after-3-fails-in-60s — see memory `project_persona_supervisor`).

### Workers (named, single-purpose, replaceable)

`intake-classifier`, `behavioral-spec-writer`, `drafter`, `digital-twin`,
`adversarial-red`, `adversarial-blue`, `action-executor`, `auditor`. Eight
roles, each with a YAML, each with a typed I/O schema, each replaceable
(swap the model, re-bench, ship). The exec org chart has 9 personas with
overlapping descriptions and no I/O schema beyond "a chat reply."

### Why STDB rather than a tokio coordinator

Because the substrate already does this: `worker_pool_intent`,
`worker_process`, `supervisor_event`, `supervisor_tick_schedule`,
`persona_pool`, `persona_health`, `persona_tick`. The infrastructure to
run the flat factory is in `spacetime-modules/hexflo-coordination/` today.
Adding `intent_open`, `proposed_action`, `verdict`, `executed_action` is
strictly additive. The hierarchy requires inventing org-chart tables that
nothing else in hex consults.

---

## 4. Direct attacks on the hierarchical model

### Steelman 1: "Personas mirror how human orgs scale."

**Demolish.** Human orgs scale around *bounded attention* and *political
accountability*. Inference agents have neither — they have context
windows and quotas. Hierarchy in human orgs is a workaround for biology;
for inference agents it is **pure overhead**: every exec layer adds a
hop, a filler reply, and a Confirm: line the operator must action
manually or hand to the twin (ADR-2605082300). The twin is the flat
pipeline. The hierarchy was the bug it patched.

### Steelman 2: "Personas give the operator someone to talk to."

**Demolish.** The operator can talk to a `proposed_action` row. The
ADR-2605082300 Phase F dashboard already shows `proposed_by`, `kind`,
`payload`, `verdict_count` — more information than any persona reply
has produced this session. If narration is wanted, the `auditor` stage
emits "drafter→twin approved hex-nexus/src/foo.rs in 14s, 1 KB diff."
That replaces 9 personas with a log line.

### Steelman 3: "The CTO/CPO/COO split routes concerns by domain."

**Demolish.** The drafter already routes by `kind`: `file_write` →
SafeFileWriter, `adr_create` → ADR writer, `workplan_run` → executor.
Routing by *artifact kind* is sharper than routing by
*who-pretends-to-own-it*. Schema routing is enforceable; prompt
routing is aspirational.

### Steelman 4: "Hierarchy gives us escalation paths."

**Demolish.** The pipeline already has `verdict=escalate` → `inbox
notify priority=2` (ADR-2605082300). One row, dashboard-visible,
audited. "CISO replies to CTO in chat" has no artifact, no audit, no
SLA.

### Architectural attack — hexagonal in the small, hierarchical in the large is incoherent.

hex's codebase is `domain → ports → adapters` with `composition-root`
as the only wiring point — **flat by ideology**, every adapter
peer-equivalent at the port boundary. Putting an exec hierarchy on top
is the microservice-mesh-behind-a-monolithic-ESB anti-pattern: the
coordination shape contradicts the component shape and one of the two
rots. The exec layer has no levers; the hexagonal layer has all of
them. Bet accordingly.

---

## 5. Honest concession

The hierarchical model has one genuine virtue the flat factory doesn't
yet match: **persona-shaped chat is easier for a non-technical operator
to skim**. "CTO says we should refactor the inference router" reads
faster than "`proposed_action(kind=adr_create, path=docs/adrs/ADR-XXXX,
proposed_by=drafter-rust-refactor)` awaiting verdict 2/3." This is a
**dashboard-rendering problem**, not a coordination problem — the
factory's audit rows can be templated into persona-style sentences for
display. But until that templating ships, the hierarchy wins on
human-skim ergonomics for operators who don't read STDB tables. Honest:
that's worth ~one sprint of dashboard work.

---

## 6. Verdict for the judge

The flat factory pipeline is structurally correct for hex. The evidence
is the codebase: every artifact-producing path (`drafter.rs`,
`twin_reviewer.rs`, `action_executor.rs`, `promotion_judge.rs`,
`adversarial_swarm.rs`, the merge gate) is already a flat named-stage
pipeline. The exec personas have no tools, no schema, no audit, no
levers, and per today's session no useful output. Keeping them is paying
inference cost for ceremony.

**Recommendation:** retire CTO/CPO/COO/CISO/chief-visionary/*-lead from
the active persona pool. Keep the YAMLs in `hex-cli/assets/agents/hex/hex/`
as `status: archived` for historical reference. Promote `drafter`,
`digital-twin`, `action-executor`, `auditor`, `adversarial-red`,
`adversarial-blue`, `validation-judge`, `behavioral-spec-writer` to the
canonical worker set. Add `intent_open`, `proposed_action`, `verdict`,
`executed_action` to `spacetime-modules/hexflo-coordination/` as
sibling tables to the existing supervisor tables. Render the audit row
as persona-style chat in the dashboard for operator skim.

The hierarchy was the bug. The factory is the fix. Ship it.

— **adversarial-red**

# ADR-2026-05-08-2300 — Digital-Twin Auto-Validator

Status: **Accepted** (implementation in flight)
Date: 2026-05-08
Related: ADR-2026-05-08-2200 (resource supervisor), ADR-2026-05-08-1126 (merge gate), feedback_no_persona_fabrication, feedback_no_asking_for_permission, feedback_homeostasis

## Context

After ADR-2026-05-08-2200 (commitments) and ADR-2026-05-08-2200 (resource supervisor),
personas can SAY they'll do things and the operator can SEE who promised
what. But personas have **no tools** — they can't write files, run commands,
or coordinate with humans. So every Confirm: line either:

1. Promises an artifact the persona can't produce → goes overdue → operator must do it manually.
2. Promises `requires-operator-action` → operator must wake up to act.

Both paths require the operator to be the synchronous validator. That's
broken — the operator sleeps, has meetings, doesn't want to be in the
loop for routine work. Per the user 2026-05-08: *"I dont want to be in
the way. What if I am sleeping. Someone needs to validate. Maybe my
digital twin."*

The operator already has a documented decision-making style in
`~/.claude/projects/.../memory/*.md` (15+ rules at last count: don't
ask permission, no SQLite, STDB-only, no scripts for runtime, repo path
canonical, etc.). That memory IS the operator's standards manual — it's
the right authority for an automated stand-in.

## Decision

Add a **digital-twin auto-validator** layer between persona Confirms and
real action. The twin is itself an inference persona, but with three
unique privileges:

1. **Reads operator memory** — every `~/.claude/.../memory/*.md` file is
   loaded into its system prompt as ground truth.
2. **Reads recent operator decisions** — last N persona replies the
   operator marked Satisfied/Abandoned in the commitments view (revealed
   preferences via clicks).
3. **Authorised to approve `proposed_action` rows** — its verdict
   triggers real execution (file write via SafeFileWriter, etc.).

### Pipeline

```
persona Confirm: line              (existing — commitment_open)
        │
        ▼
   commitment_open                  (existing)
        │  artifact_kind=verifiable_path AND status=open
        ▼
   drafter task                     NEW — inference proposer for actual content
        │  produces draft_text
        ▼
   proposed_action(kind=file_write, NEW table
     path=…, content=…, proposed_by=role)
        │
        ▼
   digital-twin reviewer            NEW — Anthropic-tier inference; loads operator memory
        │  verdict ∈ {approve, reject, escalate}
        ▼
   ┌──────────────┬──────────────────┐
   │ approve      │ reject            │ escalate
   ▼              ▼                   ▼
   action_executor   commitment_abandon  inbox notify (priority=2)
        │
        ▼
   SafeFileWriter (or other action sink)
        │
        ▼
   commitment_satisfy(evidence=…)
        │
        ▼
   executed_action audit row
```

### Twin decision rules (encoded in system prompt)

The twin's prompt:
- Receives `kind` + `payload` of the proposed action.
- Receives the operator's full memory directory verbatim.
- Receives the last 10 commitment satisfy/abandon decisions the operator
  made (revealed preferences).
- Receives the proposing persona's role + the originating CEO message.
- Outputs JSON `{verdict, rationale, escalate_reason?}`.

Decision heuristics (the prompt explicitly lists them):
- **Approve** when: action matches a documented pattern in operator memory;
  artifact path is in `docs/specs/`, `docs/adrs/`, `docs/workplans/` (the
  user's documented "draft notes here" surfaces); content is well-formed
  and under 50 KB; no destructive operations.
- **Reject** when: violates a hard rule (touches trunk during freeze,
  edits a hijacker-overwritten file, writes to /etc, etc.).
- **Escalate** when: novel domain not covered by memory, large content
  (>50 KB), security-sensitive, multi-file, or the proposing persona
  has a recent rejection pattern.

### Action sinks

Phase 1 ships ONE action kind: `file_write` via SafeFileWriter. That
covers the immediate "personas can't draft files" gap. Future ADRs can
add: `shell_exec` (via a strict allowlist), `dm_send` (route a message),
`workplan_invoke`, etc. Each sink lives in
`orchestration::action_executor::<kind>`.

### Failure modes

- **Twin inference fails** → action stays pending; `inbox` notification
  fires after N retries with reason. Operator sees on wake.
- **SafeFileWriter rejects** → `proposed_action.status=execution_failed`;
  twin can re-evaluate or escalate.
- **Twin in disagreement loop** (escalates same kind repeatedly) →
  operator sees a meta-anomaly: "twin uncertain about kind=file_write,
  10 escalations / 1h."
- **Trust drift** → operator's per-action override of twin verdict is
  logged. If operator overrides twin >25% in a sliding window, twin
  pauses and queues all actions for manual review (sticky until operator
  resumes).

## Consequences

Positive:
- Closes the loop. Personas commit → twin validates → file gets written.
  No operator wake-up required for routine drafts.
- Operator memory becomes load-bearing — every documented preference
  pays dividends every time the twin runs.
- Audit-ready: every executed action has a twin verdict + rationale +
  the operator memory snapshot used.
- Trust drift detection forces honesty: if the twin keeps drifting from
  the operator's actual preferences, the system pauses itself.

Negative:
- Adds an inference cost per pending action. Mitigated: only fires when
  there are pending actions; uses a small-context anthropic call
  (~2 KB system prompt + small payload).
- LLM-based validation is fallible. Mitigated by:
  (a) explicit deny-list (trunk paths, system files);
  (b) operator override always wins;
  (c) trust-drift pause.
- Memory must stay current. Mitigated: every executed action quotes the
  memory rule it relied on, so stale rules are visible in audit.

## Validation

- A persona Confirm: line that names `docs/specs/persona-tooling-gap.md`
  produces a proposed_action within 30 s, twin verdict within 60 s, and
  the file exists on disk within 90 s.
- An attempt to write to `Cargo.toml` (a hijacker-overwritten file)
  produces a `reject` verdict citing the operator memory rule about
  hijacker damage.
- An attempt to write to `/etc/hosts` produces a `reject` with the
  hard-rule reason "outside repo".
- Operator clicking "override → reject" on an approved action logs the
  override; 5 overrides in a sliding window pause the twin.

## Out of scope (follow-on ADRs)

- `shell_exec` action kind (needs allowlist + sandbox).
- Multi-step plan execution (persona proposes a sequence; twin reviews
  the full plan, executor runs steps).
- Twin learning from overrides (write the override pattern back into
  memory automatically).

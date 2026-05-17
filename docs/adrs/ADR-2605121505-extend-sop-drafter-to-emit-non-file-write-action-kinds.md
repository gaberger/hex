# ADR-2605121505 — Extend SOP Drafter to Emit Non-`file_write` Action Kinds

Status: **Accepted** (shipped 2026-05; commit e24fe9d5 `feat(sop): implement ADR-2605121505 — emit/execute adr_status_set kind`)
Date: 2026-05-12

## Context

The SOP self-management loop (ADR-2605082500 typed-tool library) documents seven typed primitives the persona path is meant to use:

| Tool             | Purpose                                                |
|------------------|--------------------------------------------------------|
| `file_write`     | Write content to a file (specs, drafts)                |
| `adr_draft`      | Author a new ADR (frontmatter + body)                  |
| `adr_status_set` | Flip an ADR's lifecycle status (Proposed → Accepted)   |
| `spec_draft`     | Author a behavioral spec in `docs/specs/`              |
| `code_patch`     | Apply a small code edit through the digital twin gate  |
| `workplan_emit`  | Materialise a workplan from an accepted ADR            |
| `escalate_to_operator` | Surface a blocker the SOP can't resolve          |

Today's reality (verified 2026-05-12 against `proposed_action` table):

```text
SELECT kind, COUNT(*) FROM proposed_action GROUP BY kind;
file_write | 3
(zero rows for any other kind in 3 days)
```

`hex-nexus/src/orchestration/drafter.rs` only calls `proposed_action_open("file_write", …)` (line 484, 607). `hex-nexus/src/orchestration/action_executor.rs::execute_one` only dispatches `"file_write"`; everything else hits `mark_failed("unknown action kind: …")` (line 126-138).

**Operator-observed impact (2026-05-12 board-status session):**
- Operator asked the chief-architect persona to flip ADR-2605090100 from Proposed → Accepted via `adr_status_set`.
- The ask routed to the chat-relay, got a chat-style reply, **but never produced a `proposed_action`** because the drafter has no path to emit `adr_status_set`.
- Operator had to bypass HARD RULE #11 and edit the ADR file directly.
- Same fate for the parallel ask to draft a `repo_grounding` fix ADR.

The typed-tool roster is documented in CLAUDE.md and the persona system prompts but is **unimplemented in the executor surface**. Every persona-domain change that isn't a fresh file write currently falls back to manual operator edits — which contradicts HARD RULE #0 ("EVERYTHING ROUTES THROUGH HEX").

## Decision

Extend the drafter + action_executor to emit and execute `adr_status_set` as the **first non-`file_write` kind**, establishing the pattern for the remaining five typed tools.

### 1. Commitment Type Selector

Introduce a second `artifact_kind` value the persona can emit when declaring a commitment:

```text
artifact_kind = "verifiable_path"    # existing — drafts file content
artifact_kind = "adr_status_flip"    # NEW — drafts an ADR status mutation
```

The persona's commitment row carries the target ADR id + desired status in `success_artifact` using the structured form:

```text
success_artifact = "ADR-<id>:<new_status>"
# example: "ADR-2605090100:Accepted"
```

Reason: keeps the existing commitment table schema. No new columns, no migration.

### 2. Drafter Branch

In `drafter.rs::draft_one`, after the literal-content short-circuit, check the commitment's `artifact_kind`:

```rust
match c.artifact_kind.as_str() {
    "verifiable_path"  => draft_file_write(...).await,    // existing path
    "adr_status_flip"  => draft_adr_status_set(...).await, // NEW
    other => return Err(format!("unknown artifact_kind: {}", other)),
}
```

`draft_adr_status_set` does **not** call the LLM. The persona has already made the decision in the commitment; the drafter only assembles the payload:

```json
{
  "adr_id": "ADR-2605090100",
  "new_status": "Accepted",
  "reason": "<one-line rationale pulled from the commitment.action text>"
}
```

and queues it via `proposed_action_open("adr_status_set", payload, role, commitment_id)`.

### 3. Twin Gate

Twin reviewer (`twin_reviewer.rs`) approves `adr_status_set` when:

1. `adr_id` resolves to an existing file under `docs/adrs/` (the drafter passes the canonical filename in `payload.adr_file` after a glob match — extension to drafter).
2. Current status (parsed from file) is `Proposed`.
3. Requested `new_status` is one of `Accepted | Abandoned | Superseded`.
4. `reason` is non-empty and ≤ 500 chars.

Reject (don't escalate) on any of: status already terminal, invalid transition, file not found, reason missing. The persona retries with a corrected commitment.

### 4. Executor

`action_executor.rs::execute_one` adds the dispatch arm:

```rust
"adr_status_set" => execute_adr_status_set(http, stdb_host, hex_db, repo_root, action).await,
```

`execute_adr_status_set`:
1. Resolves `payload.adr_file` (already validated by twin) relative to `repo_root`.
2. Reads the file, finds the first line matching `^Status:\s*\*\*[A-Za-z]+\*\*\s*$`.
3. Rewrites it to `Status: **<new_status>**`.
4. Inserts a new line directly after the Date line:
   ```text
   <new_status>: <today> — <reason> (autonomous via SOP, commitment <id>)
   ```
5. Writes atomically via the same final-mile guards as `execute_file_write` (path-traversal check, parent-must-exist, ≤ 100 KB).
6. Calls `proposed_action_executed(action.id)`.

Same safety envelope as `file_write` — operates only inside `repo_root`, single line mutation, no exec of external tools.

### 5. Behavioural Contract

- A persona declares a status flip by sending a board message with explicit intent (`"I commit to flipping ADR-X to Accepted because Y"`) — the chief-architect twin distils this into a commitment with `artifact_kind="adr_status_flip"`.
- The drafter sees the commitment, queues an `adr_status_set` action without LLM round-trip.
- Twin approves on the schema rules above.
- Executor mutates the file.
- Commit is left to the operator (per existing rule — autonomous loop drafts, operator commits).

## Consequences

**Unlocks**: HARD RULE #11 routing for ADR status changes; the autonomous loop can self-accept its own proposals once the chief-architect twin has reviewed.

**Sets the template** for the remaining five non-`file_write` kinds. Each new kind = (1) `artifact_kind` value, (2) drafter branch, (3) twin gate, (4) executor dispatch arm. ~150 LoC per kind.

**Risk surface**: ADR file mutations are one-line, idempotent, and reversible via git. Twin gate is strict enough that an off-spec persona can't mass-mutate ADRs.

**Open**: `adr_draft` (kind #2) needs new-file creation under a generated filename — overlaps with `file_write` but with extra constraints (frontmatter schema, next-timestamp-id allocation). Deferred to a follow-on ADR once `adr_status_set` lands and the pattern is validated.

## Validation

- `cargo check -p hex-nexus` — types align.
- Unit test in `drafter.rs`: given a commitment with `artifact_kind="adr_status_flip"` and `success_artifact="ADR-X:Accepted"`, drafter produces a `proposed_action_open` call with kind=`adr_status_set` and correct payload — no LLM call.
- Unit test in `twin_reviewer.rs`: invalid transitions (Accepted→Accepted, Proposed→Garbage) get rejected; valid (Proposed→Accepted) get approved.
- Unit test in `action_executor.rs`: a fixture ADR with `Status: **Proposed**` becomes `Status: **Accepted**` plus the dated reason line.
- End-to-end via a board ask: operator asks chief-architect to flip a test ADR; the full pipeline emits, approves, executes; the file ends up modified and the `proposed_action` row hits `status=executed`.

## Links

- ADR-2605082500 — typed-tool library spine (the roster this ADR fulfils item 3 of)
- ADR-2605082400 — personas-as-commitment-creators (the upstream contract)
- ADR-2605082300 — digital-twin auto-validator (the gate this ADR plugs into)
- Memory: `gap:sop_drafter_only_emits_file_write` (the 2026-05-12 finding that triggered this)

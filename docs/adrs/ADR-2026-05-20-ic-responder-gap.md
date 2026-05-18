# ADR-2026-05-20-ic-responder-gap

- **Status**: Proposed
- **Date**: 2026-05-20
- **Authors**: cto (drafted via SOP), gary (implemented + revised post-ship)

## Context

Today, in `hex-nexus/src/orchestration/org_responder.rs`, lines 80-85 only poll inboxes for the roles of cto/cpo/coo/ciso/chief-visionary. This limitation affects the other 26 personas listed in `hex-cli/assets/agents/hex/hex/` (dashboard-ux-architect, hex-coder, hex-fixer, hex-tester, etc.), who are registered in the persona pool and receive direct messages (DMs) that never get replies. Evidence of this issue includes:

1. A message from CTO to dashboard-ux-architect on 2026-05-15 regarding a dashboard UX issue that has been unread for 2 days.
2. Another message from CEO to dashboard-ux-architect on 2026-05-17 asking about a Kanban filter (message 126995) that remains stuck.

## Problem

The current IC responder system is limited to only five executive roles, while the rest of the organization's personas are not being addressed adequately. This gap leads to silent IC asks that remain unanswered, hindering effective communication and operational flow within the organization.

## Considered Options

1. **Widen the Responder Allowlist**: Modify the existing responder system to include all roles in the organizational chart.
2. **Add a Sister IcResponder Daemon**: Introduce a separate daemon designed specifically for handling the response of non-executive personas.
3. **Add a Per-Persona Supervisor**: Implement a per-persona supervisor that reuses the standard SOP path, tailoring the system prompt to each role using the YAML persona definitions.

## Decision

We will implement a **hybrid of Option 1 (Widen Allowlist) and Option 3
(Data-Driven from YAML)**, in a single existing daemon, instead of the
originally-proposed Option 2 (sister daemon).

### Rationale (revised after implementation)

- **No new daemon**: `org_responder.rs` already polls inboxes on a 4s
  tick with per-role fanout and a 3-slot semaphore. Spinning up a sister
  daemon would duplicate the tick loop, semaphore, and STDB query path
  with zero behavioral difference — pure overhead.
- **Data-driven, not hardcoded**: the responder loads its roster at
  startup from `parse_agent_yamls()` (already in
  `routes/org_chart.rs`), keyed off the `name:` + `role:` fields of
  every YAML in `hex-cli/assets/agents/hex/hex/`. Adding a new persona
  YAML now auto-enables responder coverage without a code change.
- **One source of truth**: `RESPONDER_ROLES`, `role_title()`, and
  `VALID_PEERS` (the @mention auto-CC allowlist) all read from the
  same `Roster` cache. No three-way drift.

### Implementation (shipped)

| Commit | What |
|---|---|
| `016174d1` | Phase 1: widen hardcoded allowlist from 5 execs to all 26 ICs (immediate unblock) |
| `b3f26ce4` | Phase 2: replace the hardcoded array with `parse_agent_yamls()` at startup; titles + peer set derive from YAML. -136 / +117 LOC. |
| n/a (YAML) | `dashboard-ux-architect.yml` was missing `role:` + `tier:` fields; one persona that was parsing as `Unknown`. Added in the same Phase 2 commit. |

### Empirical validation

Within 5 seconds of nexus restart on 2026-05-18 11:46 UTC:

- `dashboard-ux-architect` cleared its 2-day-stuck DMs (msg 126990 from
  2026-05-15 CC, msg 126995 from 2026-05-17 CEO ask) and emitted
  `Confirm: I will draft docs/specs/kanban-hide-drained-orphans.md by EOW`.
- `hex-coder` and `validation-judge` also polled their previously-ignored
  inboxes and emitted contract-conforming replies.
- `dashboard-ux-architect`'s Confirm row was consumed by the digital-twin
  loop and the spec landed on disk as autonomous commit `8fa37d0c`.

## Consequences

- Every persona with a YAML now receives `org_responder` coverage by
  default — there is no longer any "online but silent" failure mode.
- The hardcoded `marketing-*` / `research-*` role names that had been
  in `RESPONDER_ROLES` but had no backing YAMLs were dropped (they were
  unreachable anyway).
- Fallback path: if `parse_agent_yamls()` fails (e.g. dev cwd without
  the assets dir), a small static fallback covers the c-suite + leads.
  In production deployments with the rust-embed assets extracted, the
  scan always succeeds.

## References

- Code: `hex-nexus/src/orchestration/org_responder.rs`
  (`fn build_roster_from_yamls`, `fn roster`)
- Source data: `hex-cli/assets/agents/hex/hex/*.yml`
- Parser: `hex-nexus/src/routes/org_chart.rs::parse_agent_yamls()`

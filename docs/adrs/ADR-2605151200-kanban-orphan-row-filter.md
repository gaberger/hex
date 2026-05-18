# ADR-2605151200 — kanban-orphan-row-filter

- **Status**: Proposed
- **Date**: 2026-05-18
- **Authors**: cto (autonomous, via `adr_draft` typed tool)

## Context

The Mission Control kanban UI (`hex-nexus/assets/src/components/views/MissionControl.tsx`) displays `proposed_action` rows queried from SpacetimeDB's `hexflo-coordination` module. Each `proposed_action` optionally references a parent commitment via `related_commitment_id`.

When a commitment is deleted or marked obsolete (e.g., via `hex commitment retract`), orphaned `proposed_action` rows remain in STDB with stale `related_commitment_id` foreign keys. These orphans clutter the operator's kanban board and create visual noise, making it harder to triage live work.

Current workarounds:
- Manual SQL cleanup (`DELETE FROM proposed_action WHERE related_commitment_id = <id>`)
- Tolerate the noise and visually skip orphaned cards

Neither is sustainable. The operator needs a declarative filter to hide orphaned rows without mutating the database, preserving them for forensics.

**SOP telemetry hook**: This ADR serves as a smoke test for the `adr_draft` tool under ADR-2026-05-08-2500. The CTO persona should emit a well-formed ADR with Context/Decision/Consequences, a 10-digit timestamp ID, and `status: proposed`.

## Decision

Add a reactive **orphan filter** to the Mission Control kanban view:

1. **Backend (SpacetimeDB)**:
   - No schema change required. `proposed_action.related_commitment_id` already exists.
   - Optional (future): add a reducer `proposed_action_mark_orphaned(id: u64)` to soft-delete via a new `orphaned: bool` column. For MVP, detect orphans client-side.

2. **Frontend (hex-nexus/assets)**:
   - Derive orphaned status in `MissionControl.tsx`:
     ```typescript
     const orphanedIds = createMemo(() => {
       const commitmentIds = new Set(commitments().map(c => c.id));
       return proposedActions()
         .filter(pa => pa.related_commitment_id > 0 && !commitmentIds.has(pa.related_commitment_id))
         .map(pa => pa.id);
     });
     const filteredActions = createMemo(() =>
       proposedActions().filter(pa => !orphanedIds().has(pa.id))
     );
     ```
   - Bind kanban cards to `filteredActions()` instead of raw `proposedActions()`.
   - Add a UI toggle (checkbox or filter dropdown) to show/hide orphans. Default: **hide**.

3. **Observability**:
   - Log orphaned row count to console on each recompute: `console.debug('[kanban] hiding N orphaned proposed_actions')`.
   - Emit a `DomainEvent` (`OrphanedProposedActionsDetected { count, ids }`) so the digital twin can surface a toast if the count exceeds a threshold (e.g., >10).

## Consequences

**Positive**:
- Operator sees a clean kanban board without manual SQL intervention.
- Orphaned rows remain in STDB for audit (root-cause analysis, replay).
- Zero backend migration cost for MVP.
- The filter is opt-out (toggle to re-show orphans for debugging).

**Negative**:
- Client-side join (`commitmentIds.has(…)`) scales O(actions × commitments). Acceptable for <1000 rows; may need STDB-side view if scale grows.
- Orphaned rows still consume STDB storage. Future ADR can add garbage collection (e.g., auto-archive orphans >30d old).

**Migration**:
- No schema change.
- Deploy frontend update; old clients tolerate orphans as before.
- Optional follow-on: STDB reducer to mark orphans server-side + `WHERE orphaned = false` in subscription query.

**Testing**:
- Unit test: mock `commitments()` and `proposedActions()`, assert `filteredActions()` excludes orphans.
- E2E: retract a commitment via `hex commitment retract <id>`, verify Mission Control hides the associated `proposed_action` card.
- Performance: seed 500 commitments + 500 actions, measure memo recompute latency (<10ms target).

**Acceptance**:
- [ ] `hex-nexus/assets/src/components/views/MissionControl.tsx` implements `orphanedIds` memo
- [ ] Kanban cards render from `filteredActions()`
- [ ] UI toggle (e.g., "Show orphaned actions") persists to localStorage
- [ ] Console logs orphan count on each filter recompute
- [ ] E2E test: retract commitment → orphaned action hidden within 1s

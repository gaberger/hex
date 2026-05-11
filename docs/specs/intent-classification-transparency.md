# Intent Classification Transparency Panel

*status*: proposed  ·  *date*: 2026-05-11

Intent Classification Transparency Panel

**Status**: Proposed  
**Owner**: cpo  
**Date**: 2025-06-05  

---

## Problem

The operator's prompts are classified by `classify_work_intent` (ADR-[PHONE]) into tiers (T1Todo, T2MiniPlan, T3Workplan), which route to different personas and execution paths. When the classifier misfires — e.g., "implement OAuth" gets scored as T2 instead of T3 — the operator has **no immediate feedback**:

- No visibility into *which rule* matched their prompt.
- No UI to override a misclassification before execution starts.
- No audit trail of classification decisions.
- No way to see patterns in classification accuracy over time.

This creates a **silent failure mode**: the operator discovers a misroute only after the wrong persona has started work, wasting time and tokens.

---

## Solution: Mission Control Intent Panel

Add a new **Intent Classification** panel to Mission Control (`hex-nexus/assets/src/components/views/MissionControl.tsx`) that surfaces:

1. **Recent Prompts Table** (last 10 prompts from STDB `inference_log` or new `prompt_classification_log` table):
   - Operator prompt (truncated, with hover tooltip for full text)
   - Classified tier (T1 / T2 / T3 badge, color-coded)
   - Matching rule label (e.g., `feature_score`, `question`, `trivial_edit`)
   - Rule signals (e.g., "+2 implement, +2 subsystem, score=4")
   - Override button (if not yet routed)

2. **Override Flow**:
   - Operator clicks **Override** on a prompt row.
   - Modal presents three tier buttons: T1 / T2 / T3.
   - Selecting a tier writes a `prompt_reclassify` row to STDB with `(prompt_id, old_tier, new_tier, operator_override=true)`.
   - The hook router checks for overrides before dispatching; if found, uses `new_tier`.

3. **Escape Hatch Indicator**:
   - When operator types `hex skip plan`, the panel shows a yellow **⚡ Fast-track** badge next to that prompt.
   - Helps the operator understand when they've consciously bypassed planning.

4. **Classification Stats** (summary card above the table):
   - "10 prompts classified today: 6 T1, 3 T2, 1 T3"
   - "2 overrides in last 24h" (click to filter table to overrides only)

---

## Observable Artifacts

### 1. New API Endpoint

**`GET /api/intent-history`**

Returns JSON:

```json
{
  "prompts": [
    {
      "id": 12345,
      "content": "implement OAuth login with refresh tokens",
      "classified_tier": "T2MiniPlan",
      "matched_rule": "work_score",
      "rule_signals": "+1 implement, +1 login, +2 oauth, score=4",
      "routed_to": "cto",
      "can_override": true,
      "override_applied": false,
      "escape_hatch_used": false,
      "created_at": "2025-06-05T14:32:10Z"
    },
    {
      "id": 12344,
      "content": "how does the planner work?",
      "classified_tier": "T1Todo",
      "matched_rule": "question",
      "rule_signals": "trailing ?",
      "routed_to": "ceo",
      "can_override": false,
      "override_applied": false,
      "escape_hatch_used": false,
      "created_at": "2025-06-05T14:28:03Z"
    }
  ],
  "stats": {
    "total_today": 10,
    "by_tier": { "T1": 6, "T2": 3, "T3": 1 },
    "overrides_today": 2
  }
}
```

**Backend** (hex-nexus):
- New handler in `hex-nexus/src/api/intent_history.rs`.
- Queries STDB `inference_log` table (or new `prompt_classification_log` if we add one).
- Joins with `prompt_reclassify` table to fetch override status.
- Limits to last 10 prompts (configurable).

---

### 2. New STDB Table: `prompt_classification_log`

```sql
CREATE TABLE prompt_classification_log (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  session_id TEXT NOT NULL,
  content TEXT NOT NULL,
  classified_tier TEXT NOT NULL,      -- 'T1Todo', 'T2MiniPlan', 'T3Workplan'
  matched_rule TEXT NOT NULL,         -- 'feature_score', 'question', etc.
  rule_signals TEXT,                  -- '+2 implement, +2 subsystem, score=4'
  routed_to TEXT,                     -- 'ceo', 'cto', etc.
  escape_hatch_used BOOLEAN DEFAULT 0,
  created_at DATETIME DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE prompt_reclassify (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  prompt_id INTEGER NOT NULL,
  old_tier TEXT NOT NULL,
  new_tier TEXT NOT NULL,
  operator_override BOOLEAN DEFAULT 1,
  created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
  FOREIGN KEY (prompt_id) REFERENCES prompt_classification_log(id)
);
```

**Write path**: `hex hook route` (in `hex-cli/src/commands/hook/mod.rs`) writes a row to `prompt_classification_log` immediately after calling `classify_work_intent`, before dispatching to the persona.

---

### 3. UI Component: Intent Panel (SolidJS)

**Location**: `hex-nexus/assets/src/components/views/MissionControl.tsx` (insert below "Pending decisions" panel, 4-col span).

**Layout**:

```tsx
{/* Intent classification — 4 cols */}
<div class="col-span-12 lg:col-span-4 space-y-2">
  <div class="text-xs uppercase tracking-wide text-gray-500">
    Intent Classification (last 10 prompts)
  </div>
  
  {/* Stats card */}
  <div class="border border-gray-800 rounded bg-gray-900/40 p-3 text-xs">
    <div class="text-gray-300">
      {stats().total_today} prompts today: 
      <span class="text-cyan-400 ml-1">{stats().by_tier.T1} T1</span>,
      <span class="text-yellow-400 ml-1">{stats().by_tier.T2} T2</span>,
      <span class="text-orange-400 ml-1">{stats().by_tier.T3} T3</span>
    </div>
    <Show when={stats().overrides_today > 0}>
      <div class="text-yellow-300 mt-1">
        {stats().overrides_today} overrides applied
      </div>
    </Show>
  </div>

  {/* Prompts table */}
  <div class="border border-gray-800 rounded bg-gray-900/40 divide-y divide-gray-900 max-h-96 overflow-y-auto">
    <For each={prompts()}>{(p) => (
      <div class="p-2">
        <div class="flex items-center gap-2 text-xs mb-1">
          <span class={tierBadge(p.classified_tier)}>{p.classified_tier.replace(/T(\d+).*/, 'T$1')}</span>
          <span class="text-gray-500">{p.matched_rule}</span>
          <Show when={p.escape_hatch_used}>
            <span class="text-yellow-400">⚡</span>
          </Show>
        </div>
        <div class="text-xs text-gray-300 truncate max-w-xs" title={p.content}>
          {p.content}
        </div>
        <div class="text-xs text-gray-500 mt-0.5">{p.rule_signals}</div>
        <Show when={p.can_override && !p.override_applied}>
          <button
            class="mt-1 px-2 py-0.5 rounded bg-yellow-900 hover:bg-yellow-800 text-yellow-300 text-xs"
            onClick={() => openOverrideModal(p.id)}
          >
            Override
          </button>
        </Show>
        <Show when={p.override_applied}>
          <div class="text-yellow-300 text-xs mt-1">✓ Override applied</div>
        </Show>
      </div>
    )}</For>
  </div>
</div>
```

**Interaction**:
- Clicking **Override** opens a modal with three buttons: T1 / T2 / T3.
- Selecting a tier POSTs to `/api/intent-history/override` with `{ prompt_id, new_tier }`.
- Backend writes to `prompt_reclassify`, responds with `{ ok: true }`.
- UI refetches `/api/intent-history` to reflect the new override badge.

---

### 4. Hook Router Integration

**File**: `hex-cli/src/commands/hook/mod.rs`

**Change**: After `classify_work_intent` returns, log to STDB before dispatching:

```rust
let tier = classify_work_intent(&lower);
let matched_rule = find_matching_rule(&lower); // helper that returns rule label + signals

// Write classification log
let stdb_client = StdbClient::new()?;
stdb_client.execute(
    "INSERT INTO prompt_classification_log (session_id, content, classified_tier, matched_rule, rule_signals, escape_hatch_used)
     VALUES (?, ?, ?, ?, ?, ?)",
    &[
        session_id,
        &prompt_text,
        &tier.to_string(),
        &matched_rule.label,
        &matched_rule.signals_desc,
        &(lower.contains("hex skip plan")),
    ],
)?;

// Check for override
if let Some(override_tier) = stdb_client.query_override_for_prompt(prompt_id)? {
    tier = override_tier;
}

// Continue with dispatch...
```

**New helper**: `find_matching_rule(&str) -> RuleMatch` that walks `WORK_INTENT_RULES` and returns the first match, plus a human-readable signal description (e.g., "+2 implement, +2 subsystem, score=4").

---

## Success Criteria

1. **Visibility**: Operator can see the last 10 prompts + their tier + matching rule in Mission Control within 1s of typing the prompt.
2. **Override**: Operator can click **Override**, pick a new tier, and the next dispatch uses that tier (verified by checking STDB `prompt_reclassify` table).
3. **Escape Hatch**: When operator types `hex skip plan`, the panel shows a yellow ⚡ badge next to that prompt.
4. **Stats**: Panel shows "X prompts today: Y T1, Z T2, W T3" and "N overrides applied" summary.
5. **Zero regression**: Classification logic (`classify_work_intent`) unchanged; only adds logging + override-check step.

---

## Files Involved

- **hex-nexus/src/api/intent_history.rs** (new): GET + POST handlers for intent history + overrides.
- **hex-nexus/src/api/mod.rs**: Register new routes.
- **hex-nexus/src/db/schema.sql**: Add `prompt_classification_log` + `prompt_reclassify` tables.
- **hex-cli/src/commands/hook/mod.rs**: Insert logging after `classify_work_intent`, add override check before dispatch.
- **hex-nexus/assets/src/components/views/MissionControl.tsx**: New 4-col "Intent Classification" panel with table + override modal.
- **hex-nexus/assets/src/services/rest-client.ts**: Add `getIntentHistory()` + `overridePrompt(id, tier)` helpers.

---

## Open Questions

1. **Retention**: How long to keep `prompt_classification_log`? Proposal: 7 days, then vacuum.
2. **Permissions**: Should override be restricted to CEO, or available to all operators? Proposal: all operators (trust model).
3. **Escape-hatch signals**: Should we also flag `hex: skip plan` (with colon)? Proposal: yes, add to regex.

---

## Non-Goals

- **Not** a full prompt-history browser (that's a separate feature).
- **Not** a classifier *training* UI (we tune via unit tests in `hook/mod.rs`).
- **Not** a way to *edit* the prompt after classification (operator would just type a new prompt).

---

## Operator Flow Example

1. Operator types: `"implement OAuth login with refresh tokens"`
2. Classifier scores it: T2 (work_score: +1 implement, +2 oauth, score=3).
3. Mission Control Intent Panel shows:
   ```
   [T2] work_score  
   "implement OAuth login with refresh tokens"  
   +1 implement, +2 oauth, score=3  
   [Override]
   ```
4. Operator realizes this is actually a multi-layer feature (should be T3).
5. Operator clicks **Override** → T3.
6. Backend writes `prompt_reclassify(prompt_id=12345, old_tier=T2, new_tier=T3)`.
7. Next dispatch uses T3 → routes to CTO with full workplan flow.
8. Panel now shows:
   ```
   [T3] work_score (overridden)  
   "implement OAuth login with refresh tokens"  
   ✓ Override applied
   ```

---

## Implementation Phases

### Phase 0: STDB Schema (P0.1)
- Add `prompt_classification_log` + `prompt_reclassify` tables to `hex-nexus/src/db/schema.sql`.
- Migration: `hex-nexus/src/db/migrations/202506050001_intent_classification.sql`.

### Phase 1: Logging (P1.1)
- `hex-cli/src/commands/hook/mod.rs`: Insert row into `prompt_classification_log` after `classify_work_intent`.
- Add `find_matching_rule(&str) -> RuleMatch` helper that returns label + signals.

### Phase 2: API Endpoint (P2.1)
- `hex-nexus/src/api/intent_history.rs`: Implement `GET /api/intent-history` (queries last 10 prompts + stats).
- `hex-nexus/src/api/intent_history.rs`: Implement `POST /api/intent-history/override` (writes to `prompt_reclassify`).

### Phase 3: UI Panel (P3.1)
- `hex-nexus/assets/src/components/views/MissionControl.tsx`: Add 4-col Intent Classification panel with table + stats card.
- `hex-nexus/assets/src/services/rest-client.ts`: Add `getIntentHistory()` + `overridePrompt(id, tier)`.

### Phase 4: Override Integration (P4.1)
- `hex-cli/src/commands/hook/mod.rs`: Check `prompt_reclassify` table before dispatching; if override exists, use `new_tier`.

---

## Acceptance Tests

1. **Logging**: Type a prompt → check STDB `prompt_classification_log` contains row with correct `classified_tier`, `matched_rule`, `rule_signals`.
2. **API**: `curl http://localhost:13338/api/intent-history` returns JSON with last 10 prompts + stats.
3. **UI**: Load Mission Control → Intent panel shows prompts with tier badges + rule labels.
4. **Override**: Click Override on a T2 prompt → select T3 → panel shows "✓ Override applied" → next dispatch uses T3.
5. **Escape hatch**: Type `"hex skip plan: build the pipeline"` → panel shows yellow ⚡ badge next to that prompt.

---

## Maintenance

- **Vacuum**: Cron job (or STDB trigger) deletes `prompt_classification_log` rows older than 7 days.
- **Stats refresh**: Mission Control polls `/api/intent-history` every 5s (same cadence as main `/api/mission-control`).

---

## Rollout

1. Merge Phase 0 (schema) → run migration.
2. Merge Phase 1 (logging) → verify STDB rows appear.
3. Merge Phase 2 (API) → test with `curl`.
4. Merge Phase 3 (UI) → verify panel appears in Mission Control.
5. Merge Phase 4 (override) → end-to-end test with operator override.
6. Ship to prod.

---

## Related ADRs

- **ADR-[PHONE]**: Tier-based routing (T1/T2/T3).
- **ADR-[PHONE]**: Hook router implementation (`hex hook route`).

---

## Metrics

- **Classification accuracy**: % of prompts overridden (target: <10%).
- **Latency**: Time from prompt submission to panel refresh (target: <1s).
- **Usage**: % of operators who use Override feature weekly (target: >20% in first month).

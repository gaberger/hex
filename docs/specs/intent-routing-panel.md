# Intent Routing Telemetry Panel (Mission Control)

*status*: proposed  ·  *date*: 2026-05-11

Intent Routing Telemetry Panel (Mission Control)

**Status:** Proposed  
**Author:** cpo  
**Date:** 2025-05-10  

---

## Problem

The operator sends natural-language prompts to hex. Behind the scenes, `hex hook route` invokes `classify_work_intent(prompt) → Tier` (T1/T2/T3) and assigns the prompt to a persona or the twin.

**Opacity challenge:** The operator cannot see *why* a prompt routed to T1 vs T3, or whether the classifier is confident. When a T2 mini-plan unexpectedly triggers, or a T3 feature-plan is *not* triggered, the operator has no telemetry to understand the decision.

**User stories:**
1. Operator submits `"add OAuth to the portal"` and expects a T3 workplan, but sees a T1 confirmatory reply. The classifier scored it T1 because it didn't recognize "OAuth" as a subsystem signal. The operator has no insight into *why*.
2. Operator wants to tune the classification rules, but the only ground-truth is reading test cases in `hook/mod.rs:3071+`. Live traffic data is invisible.
3. Operator submits multiple prompts in a session and wants to review routing history to understand tier distribution.

---

## Decision

Add a new **Intent Routing Telemetry** panel to Mission Control (`/mission-control` → `#routing` anchor or collapsible card) that surfaces:

### 1. Real-time classification stream (last 20 prompts)

| Timestamp | Prompt (truncated) | Tier | Confidence | Signals Matched | Model/Heuristic |
|-----------|-------------------|------|------------|----------------|-----------------|
| 12:34:56  | "add OAuth to portal" | T1 | 0.3 (low) | escape_hatch=no, question=no | heuristic |
| 12:36:01  | "build end-to-end audit pipeline" | T3 | 0.9 (high) | feature_verb=build, subsystem=audit | heuristic |
| 12:37:12  | "fix typo in README" | T1 | 1.0 | trivial_edit=yes | heuristic |

**Columns:**
- **Timestamp:** HH:MM:SS relative to session start
- **Prompt:** First 60 chars, hover shows full text
- **Tier:** T1/T2/T3 badge (color-coded: T1=green, T2=yellow, T3=orange)
- **Confidence:** 0.0–1.0 score (for future ML-based classifier; heuristic returns `1.0` if rule matched, `0.3` if default fallback)
- **Signals Matched:** Comma-separated list of rule labels that fired (e.g., `trivial_edit`, `feature_verb`, `subsystem_noun`)
- **Model/Heuristic:** "heuristic" initially; future: "gpt-4o-mini" or "hex-intent-v2" when we move to ML

### 2. Tier distribution pie chart (session lifetime)

```
T1 (Todo):      67%  (20/30 prompts)
T2 (Mini-plan):  20%  (6/30)
T3 (Workplan):   13%  (4/30)
```

Visual: Pie chart with T1/T2/T3 segments, hover shows absolute counts.

### 3. Misclassification annotation (future)

Operator can click a row and mark "should have been T2" or "should have been T3." This writes to an `intent_feedback` STDB table for future classifier training. For MVP, the button is present but shows "feedback collected, not yet integrated."

### 4. Rule debugger (collapsible detail)

When the operator clicks a row, expand to show:
- Full prompt text
- Each rule evaluated (`ClassifierRule.label`) with precedence, signals, and match result (✓/✗)
- Final tier decision + rationale (e.g., "P1 trivial_edit matched → T1")

This mirrors the internal precedence-ordered evaluation in `classify_work_intent`.

---

## Behavior

### Data flow

1. **Capture:** `hex hook route` already logs `UserPromptSubmit` to STDB with `tier` and `assigned_to`. Extend this row to include:
   - `classifier_version` (string, e.g., "heuristic-v1")
   - `confidence` (f32, 0.0–1.0)
   - `signals_matched` (JSON array of rule labels)
   - `prompt_text` (string, first 1000 chars of the user's input)

2. **Storage:** New STDB table `intent_classification_log`:
   ```rust
   #[spacetimedb::table(name = intent_classification_log)]
   pub struct IntentClassificationLog {
       #[primarykey]
       #[autoinc]
       pub id: u64,
       pub session_id: String,
       pub timestamp: Timestamp,
       pub prompt_text: String,        // truncated to 1000 chars
       pub tier: String,               // "T1", "T2", "T3"
       pub confidence: f32,
       pub signals_matched: String,    // JSON array serialized
       pub classifier_version: String,
   }
   ```

3. **API:** New endpoint `GET /api/intent-routing` returns:
   ```typescript
   {
     recent: IntentClassificationRow[],  // last 20, newest first
     tier_distribution: { T1: number, T2: number, T3: number },
     session_id: string,
     session_start_ts: string
   }
   ```

4. **UI:** New `IntentRoutingPanel` component in `hex-nexus/assets/src/components/views/IntentRoutingPanel.tsx`:
   - Fetches `/api/intent-routing` on 5s cadence (matches Mission Control's existing poll rate)
   - Renders table + pie chart
   - Expandable detail row for rule debugger
   - "Mark misclassified" button per row (disabled for MVP; tooltip: "Feedback collection coming in v0.3")

5. **Integration:** Add `IntentRoutingPanel` to Mission Control as a collapsible card below "Recent Activity."

---

## Observable Artifacts

### UX
- **Mission Control route `#/mission-control`:** New collapsible card "Intent Routing Telemetry" appears below "Recent Activity."
- **Operator submits a prompt:** Within 5s, a new row appears in the routing telemetry table with tier, confidence, and signals.
- **Operator clicks a row:** Expands to show rule-by-rule evaluation trace.
- **Tier distribution chart:** Updates in real-time as new prompts arrive.

### Code
- **STDB schema:** `spacetime-modules/hex-nexus/src/lib.rs` gains `intent_classification_log` table.
- **Classifier instrumentation:** `hex-cli/src/commands/hook/mod.rs::classify_work_intent` returns `(Tier, ClassificationMetadata)` where `ClassificationMetadata` includes confidence + signals. A new `log_classification(prompt, tier, meta)` helper writes to STDB.
- **API handler:** `hex-nexus/src/rest_api/routes/intent_routing.rs` implements `GET /api/intent-routing`.
- **UI component:** `hex-nexus/assets/src/components/views/IntentRoutingPanel.tsx` renders the panel.
- **Mission Control integration:** `hex-nexus/assets/src/components/views/MissionControl.tsx` imports and renders `<IntentRoutingPanel />`.

---

## Success Criteria

1. **Visibility:** Operator can see the last 20 prompts routed, with tier assignments and matched signals, within 5s of submission.
2. **Debuggability:** Operator can expand a row and see which rules fired (✓) and which didn't (✗), explaining *why* a prompt became T1 vs T3.
3. **Trend awareness:** Operator can glance at the pie chart and see session-wide tier distribution (e.g., "80% T1 today, 10% T3 — mostly conversational").
4. **No latency penalty:** Classification logging adds <5ms to `hex hook route` invocation (fire-and-forget STDB write).

---

## Non-Goals (for this spec)

- **Operator-driven re-classification:** The "mark misclassified" button is a placeholder; actual feedback-loop integration (retraining, rule updates) is deferred to a future spec.
- **Historical retention:** Logs are session-scoped; cross-session analytics (e.g., "show me all T3 prompts from last week") are out of scope.
- **ML classifier:** This spec instruments the existing heuristic classifier. Switching to an ML model is a separate effort.

---

## Implementation Notes

### ClassifierRule extension

Current `ClassifierRule` struct:
```rust
pub struct ClassifierRule {
    pub label: &'static str,
    pub tier: Tier,
    pub precedence: u8,
    pub signals: &'static [&'static str],
    pub matches: fn(&str) -> bool,
}
```

Change `classify_work_intent` signature:
```rust
pub fn classify_work_intent(prompt: &str) -> (Tier, ClassificationMetadata)

pub struct ClassificationMetadata {
    pub confidence: f32,
    pub signals_matched: Vec<&'static str>,
    pub classifier_version: &'static str,
}
```

Evaluation trace:
- Walk `WORK_INTENT_RULES` in precedence order.
- For each rule, if `matches(prompt)` returns `true`, record `rule.label` in `signals_matched`.
- If a rule fires, return its tier + confidence=1.0.
- If no rule fires, return default T1 + confidence=0.3.

### API response shape

```json
{
  "recent": [
    {
      "id": 42,
      "timestamp": "2025-05-10T12:34:56Z",
      "prompt_text": "add OAuth to the portal",
      "tier": "T1",
      "confidence": 0.3,
      "signals_matched": [],
      "classifier_version": "heuristic-v1"
    }
  ],
  "tier_distribution": { "T1": 20, "T2": 6, "T3": 4 },
  "session_id": "sess-abc123",
  "session_start_ts": "2025-05-10T12:00:00Z"
}
```

### UI mockup (pseudo-TSX)

```tsx
<div class="border border-gray-700 rounded p-4 mb-4">
  <h2 class="text-xl font-bold mb-2">Intent Routing Telemetry</h2>
  <div class="grid grid-cols-2 gap-4">
    <div>
      <table class="w-full text-sm">
        <thead>
          <tr><th>Time</th><th>Prompt</th><th>Tier</th><th>Confidence</th><th>Signals</th></tr>
        </thead>
        <tbody>
          <For each={data().recent}>
            {(row) => (
              <tr onClick={() => toggleDetail(row.id)}>
                <td>{fmtTime(row.timestamp)}</td>
                <td title={row.prompt_text}>{row.prompt_text.slice(0,60)}…</td>
                <td><span class={tierBadge(row.tier)}>{row.tier}</span></td>
                <td>{row.confidence.toFixed(2)}</td>
                <td>{row.signals_matched.join(", ")}</td>
              </tr>
            )}
          </For>
        </tbody>
      </table>
    </div>
    <div>
      <PieChart data={data().tier_distribution} />
    </div>
  </div>
  <Show when={expandedRow()}>
    <div class="mt-4 p-4 bg-gray-900 border border-gray-700 rounded">
      <h3>Rule Evaluation Trace</h3>
      <pre>{JSON.stringify(ruleTrace(), null, 2)}</pre>
    </div>
  </Show>
</div>
```

---

## Dependencies

- **STDB schema migration:** Add `intent_classification_log` table to `spacetime-modules/hex-nexus/src/lib.rs`.
- **Classifier refactor:** `classify_work_intent` must return metadata. This is a signature change; all call sites in `hex-cli/src/commands/hook/mod.rs` must be updated.
- **API route:** New Axum handler in `hex-nexus/src/rest_api/routes/`.
- **SolidJS component:** New `.tsx` file + import into `MissionControl.tsx`.

---

## Future Extensions

1. **Operator-driven re-classification:** Let the operator mark "this should have been T3" and surface that feedback to the classifier training pipeline.
2. **Cross-session analytics:** "Show me all T3 prompts from the last 7 days" query interface.
3. **ML classifier integration:** Replace heuristic with a fine-tuned model; surface confidence scores from the model's logits.
4. **Prompt similarity clustering:** Group similar prompts (e.g., "all prompts about OAuth") and show tier consistency.

---

## Questions for Operator

- **Privacy concern:** Prompt text is logged to STDB. Should we truncate at 100 chars instead of 1000, or allow opt-out?
- **Retention policy:** Session-scoped only, or retain for 7 days?
- **Pie chart vs bar chart:** Pie chart is more compact; bar chart shows absolute counts better. Preference?

---

## References

- ADR-2605082800: Work intent classification (defines `classify_work_intent` heuristic)
- `hex-cli/src/commands/hook/mod.rs:2812+` — current classifier implementation
- `hex-nexus/assets/src/components/views/MissionControl.tsx` — existing Mission Control UI

---

**End of Spec.**

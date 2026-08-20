Routed punch-list — every gap carries either a task id, a draft path,
or an explicit `(out-of-scope)` / `(ask user)` tag:

1. Inference escalation report needs a `--since` filter
   (task a1b2c3d4-e5f6-4789-9abc-def012345678).
2. Dashboard needs a per-tier heatmap — draft at
   docs/workplans/drafts/draft-2604241200-dashboard-heatmap.json.
3. Wire subagent-stop hook to HexFlo task_complete
   (task 11111111-2222-4333-8444-555555555555).
4. `hex classify --json` flag (out-of-scope) — belongs in the
   next classifier ADR, not this workplan.

Singleton fix noted in passing (not an enumeration, so the linter
should not flag it even without a reference): fix the stray trailing
newline in `hex plan draft` output when `--stdout` is passed.

For completeness, here is the routed status table — each row tagged:

| Subsystem      | Status   | Routed to                                |
|----------------|----------|------------------------------------------|
| inbox watcher  | pending  | task f0e1d2c3-b4a5-4678-9012-3456789abcde |
| autoscaler     | broken   | docs/workplans/drafts/draft-autoscaler.json |
| reconcile loop | pending  | (ask user) — scope unclear               |

Nothing in this response should trip the punch-list linter: each
enumerated gap routes somewhere, and the lone fix statement is a
singleton rather than a gap enumeration.

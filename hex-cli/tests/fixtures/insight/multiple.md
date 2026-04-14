First observation:

★ Insight ─────────────────────────────────────
id: insight-multi-a
kind: ActionableGap
content: |
  `hex plan reconcile` doesn't detect deleted files — a task with an
  empty `files[]` silently passes reconcile.
route_to: Workplan
estimated_tier: T2
depends_on: []
─────────────────────────────────────────────────

Then, separately:

★ Insight ─────────────────────────────────────
id: insight-multi-b
kind: FailureMode
content: |
  The observe() hook swallows extraction errors without a structured
  event — only stderr. Observability gap.
route_to: Memory
estimated_tier: T1
depends_on: [insight-multi-a]
─────────────────────────────────────────────────

That's all.

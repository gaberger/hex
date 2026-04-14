Looking at the hook handler...

★ Insight ─────────────────────────────────────
The brain daemon is silently dropping queue items whose JSON schema has
`kind: workplan` when the referenced file is missing. No telemetry, no
stderr — they just vanish. This is why the queue appears to drain but
workplans never execute.
─────────────────────────────────────────────────

So the fix is to log and move them to a dead-letter queue.

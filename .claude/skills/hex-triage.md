---
name: hex-triage
description: Snapshot the full hex AIOS state in one pass — daemon, nexus, STDB, queue, improver, inbox, drafts, dirty git — to answer "is anything broken right now"
triggers:
  - triage hex
  - hex health check
  - is hex healthy
  - hex state snapshot
  - what's broken in hex
  - hex status full
  - audit hex
---

# hex-triage — Full hex AIOS State Snapshot

**Use this skill when**: someone asks "is hex working?", "what's broken?", or you've just sat down at a session and need to know the operator state before taking any action. This is read-only.

The hex surface has at least eight independent moving parts (sched daemon, nexus daemon, SpacetimeDB, sched queue, improver loop, inbox, plan drafts, git working tree). Any one can be wedged while the others look fine. This skill checks them in order of "if this is broken, nothing else matters."

## Step 1 — Core services

```bash
hex status
```

Look for: `hex-nexus running`, `spacetimedb running`, version string. If either daemon is down, fix that first — the rest of the surface depends on them.

## Step 2 — Sched daemon

```bash
hex sched daemon-status
```

If stopped, start it: `hex sched daemon --background --interval 30`. Without the daemon, the queue doesn't drain and the improver can't act.

## Step 3 — Improver homeostasis

```bash
hex sched improver status
```

The single number that summarizes loop health. Capture:

- **Score** (0-100). Below 40 = thrashing or stuck.
- **Top hypothesis** — what the loop wants to fix next.
- **Mean reward** in the Q-table — negative = recent actions are net-failing.
- **Dead-letter count** — non-zero means recurring failures the loop has given up on.

## Step 4 — Sched queue

```bash
hex sched queue history | head -30
```

Look for thrash patterns: same workplan id alternating `completed (auto-retried)` ↔ `failed`. That's the failure mode `feedback_salvage_after_sched_fail` warns about.

## Step 5 — Inbox (priority-2 overrides)

```bash
hex inbox query --unack 2>&1 | head -20
```

Per ADR-060, priority-2 inbox notifications override current work. If anything is unacked, surface it before recommending other actions.

## Step 6 — Plan drafts (T3 auto-decomposed)

```bash
ls docs/workplans/drafts/ 2>/dev/null | head -10
```

Drafts are T3-classified prompts that auto-created stub workplans. Old drafts (>7d) are stale and should be `hex plan drafts gc`'d.

## Step 7 — Pulse (multi-project view)

```bash
hex pulse
```

Useful when the user is working across multiple projects — flags any project that's stuck or has high decision churn.

## Step 8 — Git working tree

```bash
git status --short
```

Distinguish:
- **Tracked dirty files** — uncommitted in-progress work
- **Untracked drafts/** entries — stub workplans from T3 auto-decomposition
- **Modified workplans** — possibly auto-reconciled by the brain (cosmetic key-reorder is normal)

## Step 9 — Architecture health (optional, slower)

Only run if the previous steps surface no urgent issues:

```bash
hex analyze . --json | jq '.summary'
```

Reports boundary violations and dead code. Slower than the others — skip when you just need a quick health read.

## Synthesis

Output should be one paragraph per failing layer (or one line saying "all green"). Order by severity: services down → daemons down → unacked priority-2 inbox → improver thrash → queue thrash → stale drafts → dirty git. Don't list everything — only what needs attention.

## Common pitfalls

- **Treating "score 38" as a red flag in isolation** — the score is meaningful as a *trend*. Compare against `hex sched improver history`.
- **Reporting dirty workplan files as a problem** — the brain auto-reconciles JSON keys, so cosmetic diffs are expected.
- **Skipping the inbox** — priority-2 messages are load-bearing per ADR-060; they override everything else.

## ARGUMENTS

No arguments required. Run with: `/hex-triage`

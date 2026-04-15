# ADR-2604151500 — Status Line Audit: Confusing and Inaccurate

**Status:** Proposed (investigation deliverable)
**Date:** 2026-04-15
**Related:** ADR-2604150000 (brain→sched rename — drives the brain/sched naming inaccuracy in the line), ADR-2604151400 (queue list show running — same data-source disagreement issue)

## Context

The Claude Code status line currently renders as:

```
⬡ hex │ hex-intf │ ⚙ claude-Jaco2.local:85cd8bff │ ⎇ main ✱ │ ◆wp-adr-11229-algebra-p1-bazzite │ ○dev ○hex-nexus ○hex-intf 9⚡ ○eval README +3 │ ◉brain 1▶ 4⤵ │ ○db │ ◉nexus :5555 │ ◉mcp
```

User feedback (verbatim): *"this is too confusing and not accurate"*.

### Confusion symptoms (visual / semantic)

- **Glyph overload.** 14+ distinct symbols (`⬡ │ ⚙ ⎇ ✱ ◆ ○ ● ◐ ◉ ✓ ⚡ ▶ ⤵`) compete for attention with no legend. Without reading source, an operator cannot decode them.
- **Unlabeled counts.** `9⚡` displays a number with a glyph but no label. Reading the source (`hex-statusline.cjs:276`) reveals it's `agent_count` — but agent of what? Claude sessions? Background processes? Workplan tasks? The line shows the value without the question.
- **Truncation creates fake names.** `○eval README +3` parses naturally as three tokens (`eval`, `README`, `+3`) but is actually two — `eval` and `README` are project names, and `+3` is the runoff count. The truncation rule (12 chars) plus the cap-of-4 rule (`maxShow=4`) interact to produce strings that look like data but are layout artifacts.
- **Stale terminology.** `◉brain` is rendered even after ADR-2604150000 (brain→sched rename). User-facing strings still say "brain" because the rename only landed for the command surface.
- **Silent failures presented as normal state.** `○db` shows SpacetimeDB is offline using the same dim glyph used for "idle but fine". CLAUDE.md says SpacetimeDB is REQUIRED, so down should be visually loud — instead it's whisper-quiet.

### Accuracy symptoms (data correctness)

- **Disagreement with first-class commands.** Earlier this session: status line claimed `1▶ 4⤵` for sched, `hex sched status` agreed, but `hex sched queue list` showed only 2 pending. ADR-2604151400 addresses the queue-list source bug, but the status line consumed the same broken view, so it inherited (or could inherit) the same disagreement depending on which API endpoint it polled.
- **Unverified totals.** The line is sourced from `/api/brain/status` (sched), pulse-projects file, and several local probes. Whether the displayed counts are read from the same source-of-truth as `hex sched status`, `hex sched queue list`, and the dashboard has not been audited end-to-end.
- **Project-name accuracy.** With the per-project queue isolation gap (ADR-2604151330) still open, the line may aggregate counts across projects, presenting a global view as if it were project-scoped.
- **Stale agent-count semantics.** The pulse `9⚡` field is sourced from `agent_count` in the pulse-projects JSON. Whether that count reflects live agents (heartbeat within last 45s — ADR-2604058...) or all-time-registered agents is not documented in the producer or labelled in the consumer.

## Decision

**This ADR commissions an investigation, not a fix.** The deliverable is an audit report at `docs/analysis/statusline-audit-2604151500.md` containing:

1. **Per-segment provenance map** — every glyph-bearing segment in the status line traced to (a) its producer file:line, (b) the data source it polls, (c) the canonical hex command that should report the same number, (d) whether the two agree on a known fixture.
2. **Glyph legend** — exhaustive list of every symbol used, what it means, and (for ambiguous glyphs like `○`) whether the meaning differs by section.
3. **Inaccuracy log** — every disagreement found between the status line and a canonical hex command, with reproduction steps and severity.
4. **Confusion log** — every UX issue identified (truncation artifacts, missing labels, color/glyph collisions, density per character-width unit), each with a proposed mitigation.
5. **Recommendation set** — concrete changes ranked by ROI: which segments to remove, which to relabel, which to fix at the data layer, which to defer. NO implementation in this workplan; recommendations land as separate downstream ADRs.

This investigative scope matches the "ADR before code" rule and the "Insights are inputs" rule (ADR-2604142345): the investigation produces structured findings, each routed to a downstream ADR draft if action is warranted.

## Consequences

**Positive.**
- Builds an authoritative provenance map of the status line — first time this exists.
- Surfaces silent inaccuracies that operators (including the user just now) experience as "I can't trust this."
- Sets up subsequent fixes to be principled rather than spot-cleaning glyphs.

**Negative.**
- Costs investigation time without immediate visible improvement.
- Risk of bikeshed: glyph aesthetics are subjective. Mitigation: the report scores each finding by *accuracy impact* first, *confusion impact* second, and only then *aesthetic*.

## Non-goals

- **Not redesigning the status line in this ADR.** Redesign is downstream.
- **Not removing any glyph yet.** Removal proposals live in the report's recommendation section.
- **Not adding new metrics to the line.** Out of scope.

## Implementation

See `wp-statusline-audit.json`.

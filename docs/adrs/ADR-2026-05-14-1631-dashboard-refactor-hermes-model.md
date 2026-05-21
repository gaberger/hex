# ADR-2026-05-14-1631: Dashboard refactor — Hermes Agent as the model

**Status:** Proposed
**Date:** 2026-05-14
**Drivers:** The hex dashboard today ships ~20 distinct views (`Resources`, `MergeGate`, `Commitments`, `MissionControl`, `Brain`, `BrainDecisions`, `Missions`, `OpsSla`, `OrgChart`, `OrgComms`, `PersonaHealth`, `ProjectDetail`, `ProjectHierarchy`, `ActivityPanel`, `AgentFleet`, `ControlPlane`, `ADRBrowser`, `ConfigPage`, `FileTreeView`, ...). The operator tab-hops between them and there's no canonical "everything pending my attention right now" surface. `MissionControl.tsx` (commit `f4001ce5`, 2026-05-09) was the first attempt at a unified landing but stayed additive — it didn't *replace* the deep-dive views, and several capabilities shipped this session (`hex agent run`, `hex ops`, autonomous commit step, 23 autonomous commits/day) aren't surfaced anywhere. Hermes Agent solved a structurally identical problem with a single AIAgent class + one terminal/web/dashboard landing + a `/agents` activity-tree overlay. This ADR commits hex's dashboard to the same operator-attention discipline.

**Authors:** Operator (roadmap-shaped ADR; downstream per-phase ADRs and workplans land separately).

**References:**
- [Hermes Agent — Mission Control / TUI / `/agents` overlay](https://hermes-agent.nousresearch.com/docs/user-guide/tui)
- [Hermes Agent — Delegation `/agents` overlay](https://hermes-agent.nousresearch.com/docs/user-guide/features/delegation)
- ADR-2026-05-09-1200 — hex Mission Control design (initial unified landing)
- ADR-2026-05-14-1135 — hex-as-hermes-harness roadmap (this ADR is the dashboard slice of that effort)
- Commit `f4001ce5` — Mission Control single landing surface (current state of the art)
- Commit `eaeb5886` + `c96ceb57` + `2aa9c14d` — `hex agent run` simple_agent loop (the new flat path the dashboard must surface)
- Commit `f7a685b7` — `hex ops` CLI verbs (the operator-grade primitives needing dashboard parity)

## Context

### What hex has today

`hex-nexus/assets/src/components/views/` ships 20+ Solid+Tailwind components totalling ~36 K LOC. Each view is a deep-dive into a vertical:

| View | Domain |
|---|---|
| `Resources` | `/proc` walker, anomaly inbox |
| `MergeGate` | three-voter quorum on merge requests |
| `Commitments` | persona Confirm/PLAN ledger |
| `MissionControl` | unified landing (12-col grid: pending decisions, persona health, recent activity, open anomalies, top processes) |
| `Brain` / `BrainDecisions` | brain queue + decisions |
| `OrgChart` / `OrgComms` / `PersonaHealth` | c-suite topology |
| `Missions` / `OpsSla` | workplan execution + SLA tracking |
| `ADRBrowser` | ADR lifecycle browser |
| `ProjectDetail` / `ProjectHierarchy` / `FileTreeView` | per-project deep dive |
| `AgentFleet` / `ControlPlane` | fleet + cluster ops |

The natural landing is `MissionControl`. The deep-dives are valuable but should be drill-downs, not equal peers in the navigation.

### What's missing on the dashboard right now

This session shipped material the dashboard doesn't surface at all:

1. **`hex agent run` invocations.** The flat LLM-driven typed-tool agent — `eaeb5886` + downstream. Each run has a RunSummary (iterations, steps, final_text, stop_reason, elapsed_ms, per-step tool/input/output/error). The dashboard has nowhere to inspect, monitor, or kill an in-flight run.
2. **Autonomous commits.** 23 commits today with `Co-Authored-By: hex-autonomous`. The dashboard's "Recent activity" panel doesn't distinguish autonomous commits from operator commits, and doesn't link a commit to the originating `proposed_action.id` or `commitment.id`.
3. **`hex ops` operator verbs.** `write`, `send`, `abandon`. No dashboard surface for invoking them from the browser (operator has to drop to terminal).
4. **Typed-tool catalogue.** 16 tools in `ToolRegistry`. No dashboard browser for "what can the system do, what's each tool's input_schema, what's a recent successful invocation."
5. **Live agent activity tree.** When an agent run is in flight, the operator can't see the LLM iteration progression, tool calls dispatching, executor verdicts, autonomous commits landing — all in one streaming view.

### Hermes Agent's UI pattern (the target)

1. **One landing surface.** Everything that needs attention is rolled up into a prioritized feed; per-domain views are drill-downs reached from the landing.
2. **`/agents` overlay.** Live tree view of recursive `delegate_task` fan-out, grouped by parent. Per-branch cost / token / file-touched rollups. Kill and pause controls. Post-hoc review steps through each subagent's turn-by-turn history.
3. **Status line.** Persistent status line shows current state at a glance (busy/idle, active tool, last commit) on every screen.
4. **Banner paints first.** Instant first frame — the UI doesn't wait for data to be loaded before showing structure.
5. **Skill / tool catalog.** Searchable, with input_schema preview and recent invocations.
6. **Personality preview.** SOUL.md content visible from the dashboard so operators can verify identity at a glance.
7. **Multi-platform parity.** The same data is reachable from CLI, TUI, web dashboard, and messaging gateways with consistent vocabulary.

### The structural gap

The 20+ views today were built incrementally as capabilities shipped. Each one is correct in isolation. The collective deficiency is: **the operator's question "what's the system doing right now and what needs my attention" doesn't have a single answer.** `MissionControl` is the closest, but it shipped pre-`hex agent run` / pre-autonomous-commit-step / pre-`hex ops`, so the high-leverage new surfaces aren't there.

## Decision

Refactor the hex dashboard in **6 phases** with `MissionControl` as the canonical landing and the other 19 views demoted to drill-downs. Each phase is shippable on its own; phases 0-2 cover the gaps that today's `hex agent run` work surfaces; phases 3-5 are the Hermes-pattern polish.

### Phase 0 — Attention feed (smallest viable, ship next)

Goal: replace `MissionControl`'s scattered "Pending decisions / Recent activity / Open anomalies" panels with a **single prioritized attention feed** at the top of the landing.

Feed item shape:
```ts
interface AttentionItem {
  id: string;              // domain-prefixed e.g. "escalation/33069"
  priority: 0 | 1 | 2;     // 0 = blocking, 1 = decision, 2 = informational
  kind: "escalation" | "overdue_commitment" | "merge_vote_needed"
      | "resource_anomaly" | "autonomous_commit" | "agent_run_active";
  title: string;
  subtitle: string;
  age_seconds: number;
  action_url?: string;     // dashboard hash route to the deep-dive
  cli_repro?: string;      // exact `hex` verb to handle from terminal
}
```

Layout: vertical list at the top of `MissionControl`, sorted by priority then `age_seconds DESC`. Each item has a one-click "ack/inspect/handle" affordance + a copy-to-clipboard for the `cli_repro` string. The existing `MissionControl` panels (persona health, recent thoughts, top processes) move below the feed.

Backend: extends the existing `/api/mission-control` endpoint with an `attention_feed: AttentionItem[]` field. New aggregator in `hex-nexus/src/routes/mission_control.rs` (or wherever the handler lives) queries each domain's "needs attention" rows (escalated proposed_actions, overdue commitments, etc.) and merges into the unified shape.

LOC estimate: ~200 LOC TSX (new component) + ~150 LOC Rust (aggregator) + 4-6 unit tests on the priority sort.

### Phase 1 — Agent runs panel

Goal: surface every `hex agent run` invocation with live progress + post-hoc replay.

Data: new STDB table `agent_run` recording each `POST /api/agent/run` invocation:
```rust
#[table(name = agent_run, public)]
struct AgentRun {
  #[primary_key] run_id: String,           // uuid
  intent: String,
  started_at: Timestamp,
  finished_at: Option<Timestamp>,
  iterations: u32,
  stop_reason: String,
  final_text: String,
  steps_json: String,                       // serialized RunSummary.steps
  caller: String,                           // operator | scheduled | mcp | ...
  model: String,
}
```

UI: `RecentAgentRuns.tsx` panel on Mission Control showing last 10 runs with status pill, iteration count, elapsed, commit count. Click → modal with per-step trail (tool name, input JSON, output JSON, ok/error, elapsed) — Hermes' `/agents` overlay equivalent.

Live in-flight: when a run is active (`finished_at IS NULL`), SSE/WebSocket pushes step-by-step progress to the same modal.

LOC: ~80 LOC STDB schema + ~120 LOC route + ~250 LOC TSX. Wires into the existing `restClient` + subscription patterns.

### Phase 2 — Autonomous commits filter on activity feed

Goal: distinguish `Co-Authored-By: hex-autonomous` commits from operator commits, link each to its originating `proposed_action.id` + `commitment.id`.

Implementation: enrich the existing `recent_activity` panel data with `is_autonomous`, `proposed_action_id`, `commitment_id`, `tool_chain` (parsed from commit body). Render autonomous commits with a distinct badge + clickable affordance to the originating brief.

LOC: ~50 LOC backend enrichment + ~80 LOC TSX styling.

### Phase 3 — Typed-tool catalog browser

Goal: searchable catalog of the 16 typed tools with input_schema preview, description, last-5-invocations log.

Backend: `/api/tools/catalog` exports the same `ToolRegistry::anthropic_schema()` output the agent loop uses, plus per-tool invocation history from `proposed_action` (filter where `proposed_by = "tool:<name>"`).

UI: `ToolCatalog.tsx` view at `#/tools` — list with search box, click into per-tool detail showing schema (rendered with `json-pretty`), description, recent invocations table.

Hermes parity: this is the "Skill catalog" surface Hermes has.

LOC: ~100 LOC backend + ~250 LOC TSX.

### Phase 4 — Persistent status line

Goal: at the top of every dashboard view, a one-line summary of current state:

```
hex • idle • last commit 4d673b86 (3m ago) • 23 autonomous today • 0 escalations
```

Switches to busy state during in-flight agent runs:

```
hex • RUNNING agent#a3b2... iter 3/10 (cargo_check) • 17s elapsed • Esc to inspect
```

Implementation: new `<StatusLine>` component mounted in `App.tsx` shell; subscribes to a small `/api/status-line` endpoint pushed via SSE.

LOC: ~80 LOC TSX + ~60 LOC backend.

### Phase 5 — Banner-first / instant first frame

Goal: dashboard paints structure (nav, panels, headings) on first byte; data fills in as it arrives. Eliminates the current ~500-1500ms loading flicker.

Implementation: convert each panel to render a skeleton state first, then hydrate. Set explicit `<meta name="theme-color">` + favicon + static CSS hooks so the first frame is meaningful before JS bundle parses.

LOC: ~150 LOC TSX refactor across the panels + tiny CSS additions.

### Phase 6 — Drill-down demotion

Goal: the 19 deep-dive views become drill-downs from the landing, not first-class nav items. Nav consolidates to: `Mission Control` (default landing) + `Agents` (Phase 1 panel as a dedicated route) + `Tools` (Phase 3 catalog) + `Org` (collapsed c-suite/personas) + `Settings`. Everything else lives under those four.

Hash routes preserved for back-compat (`#/merge-gate`, `#/resources` etc. still work) but removed from the primary nav.

LOC: ~50 LOC nav refactor in `App.tsx`.

## Consequences

### Positive

- **Operator question "what needs my attention" has one answer.** The attention feed (Phase 0) is the single place to look.
- **The autonomous loop surfaces in the UI.** Today's `hex agent run` is invisible to the dashboard; Phase 1 fixes that.
- **Hermes ergonomics on hex substrate.** Status line, banner-first, `/agents` overlay equivalent — without giving up hex's distinguishing primitives (typed tools, c-suite topology, twin auto-approve, autonomous commit step).
- **Backward-compatible.** Every existing hash route still works; existing views aren't deleted, just demoted in the nav. No operator workflow breaks.
- **Each phase is independently valuable.** Phase 0 alone (attention feed) is the biggest single quality-of-life upgrade.

### Negative

- **TSX surface area grows before it shrinks.** New components (AttentionFeed, RecentAgentRuns, ToolCatalog, StatusLine, drill-down modals) land before any old views are deleted. Estimated +1.5-2 K LOC over the 6 phases. Phase 6 demotion doesn't delete; future phases can compact.
- **Schema additions to STDB.** Phase 1's `agent_run` table is a small migration; the hexflo-coordination module needs republish. Not breaking but is downtime if the system is mid-flight.
- **SSE / WebSocket complexity.** Phase 4 (status line) and Phase 1 (live in-flight) introduce push channels the current dashboard doesn't have. Adds a new failure mode (subscription dropped → stale status) that needs reconnection + heartbeat handling.

### Neutral

- The 19 existing views stay code-resident as drill-downs. Compaction to consolidate them into the canonical four-route nav is a separate later effort (call it Phase 7 if needed).

## Implementation

### Per-phase artifacts

Each phase ships its own commit family + maybe its own narrow ADR for the substantive ones. This roadmap ADR is the index.

### Rollout order

Strict sequence: 0 → 1 → 2 → 3 → 4 → 5 → 6. Phase 0 unblocks "operator can see what needs attention." Phase 1 unblocks "operator can see what the autonomous loop is doing." Phases 2-5 are polish. Phase 6 is the nav cleanup that should only happen AFTER the new surfaces have lived for a sprint and proven they don't need fallback paths.

### Estimated effort

| Phase | LOC | Days | Risk |
|---|---|---|---|
| 0 attention feed | ~350 | 1-2 | Low — pure addition |
| 1 agent runs panel + STDB | ~450 | 2-3 | Med — schema migration |
| 2 autonomous commits filter | ~130 | 1 | Low |
| 3 tool catalog | ~350 | 2 | Low |
| 4 status line + SSE | ~140 | 1-2 | Med — push channel |
| 5 banner-first | ~150 | 1 | Low |
| 6 nav demotion | ~50 | 0.5 | Low |
| **Total** | **~1670 LOC** | **8-12 days** | |

### Migration plan

Zero-breaking. All phases additive until Phase 6, which is nav-level reshuffling preserving hash routes. Existing operator muscle memory (drilling into `#/merge-gate`, `#/resources`, etc.) keeps working forever.

### Dogfood

Phases 0, 2, 3 are TSX-only and ideal candidates to be SCAFFOLDED via `hex agent run` (per the new flat path proven in `eaeb5886`). The pattern: operator fires `hex agent run "create a Solid component at <path> that ..."` → code_patch lands the file → cargo check / typescript check via the next-phase tools → autonomous commit. Eat our own dogfood.

### Risk + mitigation

| Risk | Mitigation |
|---|---|
| Adding push channels (Phase 4) introduces a new failure mode | Reconnect-with-backoff in the subscription layer; status line shows a degraded badge when the channel is down |
| LLM-generated TSX from dogfooding produces non-idiomatic Solid signals | All scaffolded components pass `tsc --noEmit` via cargo_check-equivalent before landing. The schema-aware prompt for `hex agent run` is extended with hex dashboard idioms (restClient, navigate, signals) |
| Operator dependency on the 19 old views during transition | Hash routes preserved indefinitely. Phase 6 is the only nav-level change; rollback is a one-line revert |
| 36k existing TSX LOC has discoverability problems for the LLM | Phase 3's tool catalog (which surfaces internal capabilities) is a model — same pattern can index existing components for the LLM in future iterations |

## What this ADR does NOT commit to

- Deleting any of the 19 existing views. They become drill-downs; the code stays.
- A specific TSX/Solid framework upgrade (we stay on Solid 1.x).
- A full design system / token refresh. Existing Tailwind classes continue to work; new components use the same.
- Mobile-first responsive design (out of scope — operator surface is desktop browser + terminal).

## Open questions (resolved in per-phase ADRs)

- Phase 1: SSE vs. WebSocket for live in-flight agent run progress? Hermes uses WebSocket; hex already has WebSocket infra via STDB subscriptions. Probably WebSocket for parity.
- Phase 4: status line content priority — what's the canonical 1-2 lines that summarize hex's state at a glance? Needs a small user study (just operator preference).
- Phase 3: tool invocation history retention — keep last 5 per tool in the catalog page, or paginate the full history? Probably last 5 in-page + link to a full history view if needed.

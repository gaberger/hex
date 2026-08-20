# Dashboard UX audit — 2026-05-22

Audit target: hex-nexus dashboard served at `http://localhost:5555/dashboard`.
Source: `/var/home/gary/hex-intf/hex-nexus/assets/src/` (Solid.js + Tailwind v4).
Auditor: Claude (operator-authorized direct execution; ux-designer worker role
is not yet wired — separately tracked in `wp-extend-hex-agent-worker-roles.json`).
Method: read-only static review of the source plus live REST-payload sampling
against the running daemon (mission-control, decisions, workplans, health,
hex-agents). No dashboard files modified.

Scope: operator-facing UX. Excludes app-server availability bugs (the
`Timestamp { __timestamp_micros_since_unix_epoch__ … }` rendering issue and
the `recent_executed[].executed_at: ""` empty timestamps are caught here
because they manifest as UX, but the underlying field-shape bugs are
engineering-lead's domain).

---

## Executive summary

**Top 3 things that hurt most:**

1. **Mission Control — the operator's landing page — drowns the conversation
   stream in duplicate anomaly cards.** Live payload right now: 15 of 15
   attention items are `resource_anomaly` rows, 5 of the first 5 are
   `rss_oversize`/`cpu_pin` duplicates. Operator sees the same warning
   cluster every 5s refresh; the actual conversation gets pushed off-screen.
   No client-side dedup, no class-collapse, no "Ack all of this kind"
   affordance.
2. **The dashboard talks in three different visual dialects.** Mission
   Control + AttentionFeed + AgentRuns use `zinc-*` palette; everything
   else uses `gray-*` (which is overridden by CSS vars for theming).
   `zinc-*` is NOT overridden — so light mode breaks on Mission Control
   only, and dark mode runs visibly cooler (slightly bluer) than the
   surrounding chrome. Pure theme leak.
3. **The global sidebar has 18 nav items and growing**, with mixed
   information density (project sub-nav at `text-[13px] py-1.5`, "Global"
   section identical, no visual chunking beyond a `border-t`). Operator
   has to read every label to find Mission Control vs. Brain vs. Missions
   vs. Brain Decisions vs. Ops SLA vs. Resources vs. Commitments vs.
   Persona Health — many of which now render the same MissionControl
   component (App.tsx lines 267-290 explicitly aliases 11 drilldown hashes
   to the same surface). The IA hasn't caught up with the
   single-surface-console refactor.

**Operator narrative.** I load the page and land on Mission Control. The
chat stream is in `zinc-950` — slightly different from the chrome's
`gray-950`, just enough to make me wonder if I'm on the right page. The
sidebar has 17+ items; half of them ("Brain", "Brain Decisions", "Merge
Gate", "Personas", "Thoughts", "Resources", "Commitments", "Missions",
"Ops SLA") all open the same component. The "Factory" rail on the left
shows persona heat tiers in colored borders, but six of the twelve
workspace-development roles ("chief-visionary", "chief-architect",
"product-lead", etc.) aren't in `persona_pool` so they render as cold
gray placeholders forever — the operator can't tell which are real. The
stream itself works, but interleaved with it are 8-15 duplicate
"resource_anomaly: rss_oversize (warn)" cards, every single one with the
same `[Ack]` button. I can dismiss them one at a time. Scrolling, the
"thinking…" italics for pending replies sit in the same bubble shape as
real replies — I can't tell at a glance if the team is working or stalled.
The compose box at the bottom is sized for one line of casual chat in a
chrome-defined chat product. This is supposed to be where I drive a 65-agent
fleet — I'd like to write a paragraph without it scrolling.

---

## Findings

### P0 (blocking)

1. **Mission Control duplicate-anomaly storm**
   *What's wrong:* `MissionControl.tsx:316-324` renders every `attention_feed`
   item as a separate inline card. Live: 15 anomalies, 14 of which are
   `resource_anomaly` of just 2 kinds (`rss_oversize`, `cpu_pin`). No client
   dedup; the suppress key `class:${kind}:${subtitle.slice(0,40)}` requires
   user click + only lasts 5min.
   *Why painful:* The stream IS the operator console; it's been buried by
   ambient noise. Real persona replies and commits scroll off screen.
   *Fix shape:* Group `attention` items by `(kind, severity, title.slice(0,30))`.
   Render a single collapsed card per group with count badge ("rss_oversize
   ×8"), expanded on click. Add an "Ack all rss_oversize" action that
   sweeps the suppress key for the whole class. Cap inline rendering at 3
   groups; the rest moves to a count chip in the top bar
   ("47 quieter alerts").
   *Severity:* P0. *Effort:* M (~half a day; client-only).

2. **`zinc-*` theme leak on Mission Control / AttentionFeed / AgentRuns**
   *What's wrong:* `MissionControl.tsx` lines 431, 433, 466, 467, 469,
   480-514, 672-688, plus all of `AttentionFeed.tsx` and `AgentRuns.tsx`,
   use `bg-zinc-950`, `bg-zinc-900`, `text-zinc-100/200/300/500/600`,
   `border-zinc-700/800/900`. The theme override layer in
   `app/dashboard.css:112-134` only overrides `bg-gray-*`, `text-gray-*`,
   `border-gray-*`. So Mission Control:
   - In **dark mode**: renders a slightly bluer surface than the rest of
     the chrome (visible at the chrome/main boundary at the top bar).
   - In **light mode**: stays dark — the page is half-light, half-dark
     (broken).
   *Why painful:* (a) Visual cohesion is destroyed; (b) light mode is
   unusable on the operator's primary surface; (c) the `data-theme="light"`
   block in `dashboard.css:69-100` doesn't even know zinc exists.
   *Fix shape:* sed/replace `zinc-` → `gray-` across the 3 files (the
   numeric scales already line up almost 1:1 in Tailwind v4). One-line PR.
   *Severity:* P0. *Effort:* S (15 min, mostly mechanical).

3. **`personas[].last_tick_at` is a raw `Timestamp { … }` object, not a string**
   *What's wrong:* Live payload returns `last_tick_at: "Timestamp {
   __timestamp_micros_since_unix_epoch__: 1779489940766495 }"`. The
   `PersonaRow` type at `MissionControl.tsx:20` declares it as `string`, and
   the Factory rail at line 480-513 doesn't render it at all — so the bug
   is hidden in MissionControl but breaks `PersonaHealth.tsx` which DOES
   format `last_tick_at` via `Date.parse`. Operator sees "Invalid Date" or
   "—" instead of "ticked 12s ago".
   *Why painful:* Hides whether personas are alive. PersonaHealth is the
   diagnosis page for "did the supervisor go to sleep?" and it shows
   nothing useful.
   *Fix shape:* either (a) fix nexus-side serializer to emit ISO-8601 (the
   real fix; engineering-lead/CTO), or (b) UI workaround in
   `MissionControl.tsx::tsToEpoch` already handles this format — extend it
   to PersonaHealth.tsx and treat as the project-wide ts parser. Until (a)
   ships, do (b).
   *Severity:* P0. *Effort:* S (UI workaround); M (proper fix in nexus).

4. **Default landing route is `mission-control` but it's invisible from
   the sidebar pattern**
   *What's wrong:* `router.ts:62` defaults to `mission-control`, but the
   sidebar's first item is "Control Plane" and Mission Control is buried
   in the "Global" section midway down (App.tsx:677-695). When the
   operator types `localhost:5555/dashboard` and lands on Mission Control,
   the sidebar highlight is in the middle of the rail — visually disorienting.
   *Why painful:* Operator can't tell where they are. Looks like they
   landed on a sub-page.
   *Fix shape:* Move "Mission Control" to the top of the sidebar above
   "Control Plane" with a distinct treatment (e.g. accent background, no
   border-left-2 but full bg-accent/15). Or make Mission Control the
   right side of the header itself so it's always one click.
   *Severity:* P0. *Effort:* S.

### P1 (high-friction)

5. **Sidebar IA is stale — 11 entries point at the same MissionControl
   component**
   *What's wrong:* App.tsx:267-290 declares 11 hashes (`#/resources`,
   `#/commitments`, `#/merge-gate`, `#/persona-health`, `#/thoughts`,
   `#/brain`, `#/brain-decisions`, `#/ops-sla`, `#/missions`,
   `#/mission-detail`, `#/agent-runs`) ALL render MissionControl. The
   sidebar still lists most of them as separate items (lines 752-885). A
   click changes the URL hash but the page doesn't visibly change. The
   "Brain" item is even more confusing — the Match clause at line 948 says
   `<Brain />`, but the `isBrainPage` is hardcoded to `false` (line 280).
   So clicking "Brain" routes to `brain`, the Switch sees `brain` and
   tries to render `<Brain />` — but `isMissionControlPage()` is true first
   (the drilldown set includes "brain"), and the full-screen MissionControl
   wins, so `<Brain />` never renders.
   *Why painful:* Operator clicks nav, nothing visibly changes. They
   conclude the dashboard is broken.
   *Fix shape:* Collapse the sidebar to: Mission Control, Control Plane,
   Projects, Research Lab, Fleet/Inference (Global), Settings.
   Remove "Brain", "Brain Decisions", "Merge Gate", "Personas",
   "Thoughts", "Resources", "Commitments", "Missions", "Ops SLA",
   "Agent Runs", "Swaps" as separate sidebar items. Surface them as
   filter chips or scroll-anchors inside Mission Control. Keep the
   bookmarkable hashes; remove only the sidebar buttons.
   *Severity:* P1. *Effort:* M (rip out ~250 lines of sidebar, but the
   underlying redirects already exist).

6. **Stream-message "thinking…" placeholder is indistinguishable from
   real replies**
   *What's wrong:* `MissionControl.tsx:557-584` renders pending bubbles
   with just `opacity-60 italic` and body `_thinking…_`. Same shape, same
   color, same width. On a busy page the eye misses it.
   *Why painful:* Operator can't tell if a persona is working or stuck.
   On slow inference (qwen 32B on CPU) "thinking…" can sit there 30s+ —
   operator concludes the system is broken and re-sends.
   *Fix shape:* (a) Render pending bubbles with a left border-l-2 accent +
   shimmer skeleton (replace literal `_thinking…_` with three pulsing
   `bg-zinc-700` skeleton lines using `animate-pulse`). (b) Add an elapsed
   counter ("waiting 12s") that ticks. (c) After 60s, swap to "(slow inference
   — qwen2.5-coder:32b CPU)" with a model-name hint pulled from
   `inference_dispatch`.
   *Severity:* P1. *Effort:* M.

7. **Factory rail shows 12 personas but only 8 are real**
   *What's wrong:* `MissionControl.tsx:411-428` synthesizes a fake
   `PersonaRow` for any workspace role NOT in `persona_pool`. Live now: 8
   real personas (sre-lead/coo/ciso/cto/cpo/...), but the rail shows 12
   in the "dev" workspace including chief-visionary, chief-architect,
   product-lead, engineering-lead, design-lead, validation-judge — at
   least 4 of those are placeholders with `last_tick_at: ""`.
   *Why painful:* Operator clicks "@chief-visionary" expecting an answer;
   nothing happens because that persona isn't registered. No visual
   signal of which is which.
   *Fix shape:* Render synthesized placeholders with a distinct treatment:
   dashed border-l-2, `opacity-50`, badge "not started" instead of the
   green/cold dot. Click should still set `@chief-visionary ` (it's a hint
   to the operator that addressing this role will spawn it), but the
   visual says "this is offline".
   *Severity:* P1. *Effort:* S.

8. **`recent_executed[].executed_at` is `""` — commit cards say "in
   1970"**
   *What's wrong:* Live payload returns 12 `recent_executed` entries all
   with `executed_at: ""`. `MissionControl.tsx:309-311` filters them out
   if `!ts`, so the inline commit cards never appear. The "factory heat"
   visualization is therefore based ONLY on chat + commitments + thoughts,
   missing the most important signal (writes happened).
   *Why painful:* Operator can't see autonomous-write activity in the
   stream — the primary proof-of-life signal is invisible.
   *Fix shape:* Backend fix at nexus serializer for `proposed_action_executed`
   timestamp (engineering-lead). UI workaround: if `executed_at` empty,
   fall back to `id` (monotonic) and tag the card with "(time unknown)"
   instead of dropping silently.
   *Severity:* P1. *Effort:* S (UI), M (server).

9. **Color-only state encoding fails WCAG 1.4.1**
   *What's wrong:* Status dots everywhere (`ControlPlane.tsx:62-79`,
   `ProjectHome.tsx:53-58`, `AgentFleet.tsx:21-27`, `MissionControl.tsx:498`)
   are a 2x2 `rounded-full` with green/yellow/red only. No shape, no text
   tag adjacent. A red-green colorblind operator (8% of men) can't tell
   "active" from "dead".
   *Why painful:* WCAG 2.1 AA 1.4.1 (Use of Color) violation. Also: the
   `bg-red-500` connection-pulse in ConnectionStatus animates — relying on
   movement to convey state fails 2.3.3 (animation from interactions).
   *Fix shape:* Add a glyph alongside every dot:
     - active → `●` green
     - stale  → `◐` amber
     - dead   → `✕` red
     - idle   → `○` gray
   Or short text suffix ("· active"). Don't rely on color alone.
   *Severity:* P1. *Effort:* M (touches ~15 components).

10. **5s polling causes per-card flicker on the attention feed**
    *What's wrong:* `MissionControl.tsx:36` polls every 5000ms; the stream
    is rebuilt from scratch each refresh (`createMemo<StreamItem[]>` at
    line 277). Solid's keyed `<For>` should diff cleanly, but the items
    don't have stable IDs — attention items use `id: a.id`, but pending
    chat items have ts-based synthetic IDs and the SOP messages have
    `msg_id` which advances. On every refresh, items at the bottom of the
    stream re-key and the eye sees a flash.
    *Why painful:* Operator reads slower because the page won't sit still.
    Eye-tracking studies put cognitive cost ~30% higher on flickering
    realtime UIs (Nielsen 1994).
    *Fix shape:* (a) Move to a STDB subscription on `org_message`,
    `proposed_action`, `proposed_action_executed`, `resource_anomaly`
    instead of polling — the connection store already subscribes to most
    of these. (b) Until then, ensure keyed `<For>` uses
    `key={(item) => `${item.kind}:${item.commit?.id ?? item.chat?.from
    ?? item.attention?.id}:${item.ts}`}`.
    *Severity:* P1. *Effort:* L (subscription path); M (key-fix workaround).

11. **Compose box is 2 rows for fleet orchestration**
    *What's wrong:* `MissionControl.tsx:674-682` is a `rows={2}` textarea
    with `resize-none`. The placeholder reads "Tell the team. Plain text
    broadcasts to the c-suite. '@cto …' addresses one. ⌘↵ to send."
    *Why painful:* Operator drives 65 agents from this box. Their asks
    are paragraphs ("audit the dashboard for UX issues — here's the
    scope:") not chat one-liners. 2 rows means they're scrolling inside a
    tiny window.
    *Fix shape:* (a) `rows={4}` default. (b) auto-grow up to `max-h-[40vh]`
    on input. (c) Show a hint chip "⌘↵ Send · Esc Cancel · ⇥ for
    suggestions". (d) Bonus: Ctrl+Up recalls last sent intent.
    *Severity:* P1. *Effort:* S.

12. **Top bar status chip "STDB ✓ / ✗" is too quiet**
    *What's wrong:* `MissionControl.tsx:450` shows `STDB ✓` in 11px. The
    ConnectionStatusBanner is the actual surface that screams when STDB
    is down (red bar at top), but it's dismissible and once dismissed
    only re-shows on state change. In `unreachable` state, after dismiss,
    the operator has only the tiny "STDB ✗" chip to see they're flying
    blind.
    *Why painful:* The whole UI keeps polling and rendering as if it
    were live, but values are stale. The operator may take destructive
    action on stale state.
    *Fix shape:* (a) Make the chip larger and red-background when ✗.
    (b) When STDB ✗, badge ALL data sections with a "stale since HH:MM"
    overlay. (c) Make the banner re-appear after 60s even if dismissed,
    while in `unreachable` state.
    *Severity:* P1. *Effort:* M.

13. **Sidebar — sub-nav icon click target is 3.5×3.5 (14px), below the
    44×44 hit-target standard**
    *What's wrong:* `App.tsx:590` sub-nav SVG is `h-3.5 w-3.5` (14px) and
    the button padding is `px-3 py-1.5` → button hit area ≈ 32px tall.
    WCAG 2.1 AAA 2.5.5 calls for 44×44; AA-extended (2.2 SC 2.5.8) calls
    for 24×24. We're at the edge of AA-extended for the button, well
    under for the icon-only collapsed state.
    *Why painful:* Touch users on iPad/tablet (operator demos) miss clicks.
    Mouse users with imprecise pointing (a11y) miss clicks.
    *Fix shape:* Bump sub-nav buttons to `py-2` (~40px tall); bump icons
    to `h-4 w-4`; in collapsed mode use full `w-12 h-10` hit area.
    *Severity:* P1. *Effort:* S.

14. **Empty-state copy is generic ("No agents", "No commits")**
    *What's wrong:* `ProjectHome.tsx:319`, `:375`, `:262` all show two-word
    empty states. No guidance on how to populate them.
    *Why painful:* New operator can't tell what to do. "No commits" — am
    I in the wrong project, or has the team genuinely shipped nothing?
    *Fix shape:* Replace each with a 2-line empty state including a CTA:
      - "No agents · Spawn one with `hex agent run <persona>` or +Spawn"
      - "No commits · The autonomous writes will appear here once an
        executor approves a code_patch (see #/missions)"
      - "No active swarms · `hex swarm init <name>` or click + New Swarm"
    *Severity:* P1. *Effort:* S.

15. **The "Factory" rail header says "click → DM" but clicking sets
    `@role ` in the compose box — that's NOT a DM**
    *What's wrong:* `MissionControl.tsx:478` advertises "click → DM",
    `:494` actually sets `setIntent(`@${p.role} `)` (a public message
    routed to one role). The operator's mental model says DM = private;
    actually the message goes to `recipients = [targetRole]` and shows in
    the public stream.
    *Why painful:* Operator may type sensitive content into "@ciso " thinking
    it's private, then see it published in the shared stream.
    *Fix shape:* Change the rail's hint from "click → DM" to "click →
    address". Or actually make it route via a true DM table
    (`personal_message`) — different work entirely (engineering-lead).
    *Severity:* P1 (safety-relevant). *Effort:* S (rename).

### P2 (polish)

16. **Heat-tier border colors collide with status-dot colors**
    *What's wrong:* `MissionControl.tsx:486-490` uses `border-l-red-600` to
    mean "very hot persona", but elsewhere `bg-red-400/500` means "failed
    / error". Operator parses left border as a state signal.
    *Fix shape:* Use a non-status color ramp for heat: amber/orange/violet
    or a saturation ramp on the brand cyan.
    *Severity:* P2. *Effort:* S.

17. **Mobile drawer covers the entire viewport but the close target is
    a backdrop click — no visible X**
    *What's wrong:* `App.tsx:497-499` backdrop tap closes; no X button on
    the drawer itself. Touch users tap the visible part of the drawer to
    close, hit a nav item instead, and accidentally navigate.
    *Fix shape:* Add an explicit X at top-right of the drawer.
    *Severity:* P2. *Effort:* S.

18. **`<svg>` icons embedded as `innerHTML={item.icon}` in App.tsx:592**
    *What's wrong:* The sub-nav icons are stored as raw SVG path strings
    in `projectSubNav[].icon`, then injected via Solid's `innerHTML`.
    This bypasses Solid's reconciler and creates a low-grade XSS surface
    (anyone who can edit the icon strings can inject script). Also fails
    the CLAUDE.md security rule "Primary adapters MUST NOT use innerHTML
    with data that originates outside the domain layer."
    *Fix shape:* Refactor `NavItem.icon` from raw string to a `Component`
    or JSX fragment; render via `{item.icon}`.
    *Severity:* P2. *Effort:* M (touches 12 nav items).

19. **No `<main>` landmark; no skip-to-content link**
    *What's wrong:* `App.tsx:921` uses `<div class="flex flex-1 flex-col
    overflow-hidden">` for the content area, not `<main>`. No skip link
    above the sidebar.
    *Why painful:* Screen-reader users tab through 17 sidebar items
    before reaching content.
    *Fix shape:* Wrap content in `<main id="main">`. Add hidden-until-focus
    skip link `<a href="#main" class="sr-only focus:not-sr-only">Skip to
    main content</a>` as the first child of `<body>` / App.
    *Severity:* P2. *Effort:* S.

20. **No live region for new attention items or persona replies**
    *What's wrong:* Only one `role="alert"` exists in the codebase
    (Brain.tsx:507). New chat replies and new critical anomalies are
    invisible to screen readers.
    *Fix shape:* Add `<div aria-live="polite" class="sr-only">…</div>`
    to MissionControl that mirrors the latest stream item's summary
    ("ciso replied: …" / "new critical anomaly: rss_oversize").
    *Severity:* P2. *Effort:* S.

21. **Toast container is `min-w-[280px]` but appears bottom-right — and
    BottomBar (the compose box on non-MissionControl pages) is bottom-full**
    *What's wrong:* On pages with `BottomBar` (e.g. project pages),
    toasts overlap the input row. The 4s auto-dismiss helps, but during
    those 4s users can't see what they're typing on long messages.
    *Fix shape:* Anchor toasts to `bottom-20` when BottomBar is present;
    or move to top-right.
    *Severity:* P2. *Effort:* S.

22. **Header logo + "HEX NEXUS" is a button but only shows a hover
    color-change — no visual affordance that it's clickable**
    *What's wrong:* `App.tsx:443-445` is `<button>HEX NEXUS</button>` that
    routes home. No underline, no chevron, no cursor hint beyond default.
    *Fix shape:* Add `cursor-pointer` (the focus ring is already there)
    and subtle hover treatment (e.g. opacity 80→100 plus underline on
    "HEX NEXUS"). Or just don't make it a button — keep it as a label
    and use the Control Plane sidebar item for navigation.
    *Severity:* P2. *Effort:* S.

23. **Multiple polling intervals (4s, 5s, 8s, 10s, 15s, 30s) — no
    visibility into pacing**
    *What's wrong:* See grep output — MergeGate=4s, MissionControl=5s,
    Commitments=5s, OpsSla=8s, Missions=10s, Brain=10s/15s,
    BrainDecisions=30s. Operator can't tell which surface is "live" vs
    "slow refresh".
    *Fix shape:* Standardise to two tiers — "live" (subscription, no
    poll) and "slow" (15s). Show the actual interval as a label
    (Resources.tsx already does this at line 143).
    *Severity:* P2. *Effort:* M (or wait for the subscription migration in
    #10).

---

## Recommended remediation order

**Ship in this order. These are the 5 highest-ROI fixes:**

1. **#2 — `zinc-*` → `gray-*` global replace** (15 min). Single biggest
   visible cohesion win. One commit, mechanical.
2. **#1 — Mission Control anomaly grouping** (half-day). The operator's
   primary stream becomes readable. Highest functional impact.
3. **#4 — Mission Control to top of sidebar with distinct treatment**
   (30 min). Fixes "where am I?" disorientation.
4. **#5 — Collapse the 11 dead sidebar items into Mission Control filter
   chips** (1 day). The sidebar IA finally matches the post-refactor
   reality. Cuts cognitive load on first paint by ~50%.
5. **#6 — "thinking…" skeleton + elapsed counter** (half-day). Operator
   can see whether the team is working.

**Estimated total effort for the top-5: ~2.5 days of focused work.** All
are within hex-ux's bounded primary-adapter scope (no domain/ports changes).

**Suggested implementation:** hex-ux (Path C autonomous) for #2, #4, #6,
#11, #13, #14, #16, #21, #22 — all are pure CSS / JSX tweaks inside one
primary adapter. For #1, #5, #10, #20 use a Claude in-session subagent
because they require structural refactor + careful diff (item rekeying,
sidebar IA collapse, STDB subscription wiring). For #3, #8 backend
field-shape fixes, route a board ask to engineering-lead (Timestamp
serialization, `executed_at` field).

**Workplan suggestion:** create `wp-dashboard-ux-remediation-2026-05-22.json`
with two phases:

- **Phase 1 — Cohesion & Clarity (P0):** items #1, #2, #3, #4 (#3 ships
  as both UI workaround AND a sub-task asking engineering-lead for the
  proper fix).
- **Phase 2 — Operator Console Discipline (P1):** items #5, #6, #9, #10,
  #11, #12, #14, #15.
- Defer all P2 items to a follow-up workplan; they're polish, not
  friction.

---

## Out of scope

- **Authentication / login UI.** Dashboard currently has none — STDB ws
  connect happens on page load with localStorage tokens. Auth surface is
  pending on ADR-2026-05-09-1100-multi-host-substrate-composition
  (currently aging in Proposed >5 days — separate issue, flagged in
  /api/decisions).
- **Secrets management UI.** No surface exists; `secret_grant` is
  CLI-only. Out of scope until there's a screen to audit.
- **Research Lab page** (`#/research-lab`, ResearchLab.tsx). Lightly
  sampled; appears to be a config + sweep summary view. Did not deep-read.
- **OrgChart / OrgComms / TeamDashboard** full-screen pages. Each is its
  own >400-line module; they would benefit from the same `gray-*`/`zinc-*`
  audit and 5-step sidebar collapse but I didn't drill in.
- **The dashboard's own light-mode CSS.** dashboard.css does light-mode
  overrides for gray; verifying every component renders correctly in
  light mode is a session of its own.
- **Performance under high agent counts.** Did not measure render time
  with 65 agents + 95 workplans + 15 attention items live. The MissionControl
  `stream` memo recomputes the entire array each 5s — likely OK at current
  scale but should be profiled before the fleet grows to 200+.
- **Backend serializer bugs** that surface as UX (`Timestamp { … }`
  rendering, `executed_at: ""`). Flagged here; the actual fix lives in
  hex-nexus, not the dashboard.

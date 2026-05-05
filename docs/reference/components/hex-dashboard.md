# Component: hex-dashboard

## One-Line Summary

Developer control plane for multi-project hex management — Solid.js + Tailwind UI served by hex-nexus, real-time via SpacetimeDB subscriptions.

## Key Facts

- Solid.js + Tailwind, source under `hex-nexus/assets/src/`.
- Bundled into the hex-nexus binary via `rust-embed`. There is no separate dashboard process.
- Served at `http://localhost:5555/` once `hex nexus start` is up.
- Real-time updates come from SpacetimeDB WebSocket subscriptions, not from nexus SSE.
- Editing assets requires a binary rebuild + nexus restart + hard browser refresh (Cmd-Shift-R / Ctrl-Shift-R).

## Build + dev loop

```bash
# 1. Edit hex-nexus/assets/src/...
# 2. Rebuild the bundle and re-embed:
cd hex-nexus && cargo build --release
# 3. Restart nexus:
hex nexus restart
# 4. Hard-refresh the browser
```

Or use the `hex-dev-rebuild` skill which automates 1→4.

## API Surface (panels)

The dashboard surfaces several views, each backed by SpacetimeDB subscriptions and/or nexus REST calls.

| Panel | Source data | File |
|-------|-------------|------|
| Project list | SpacetimeDB `project` + `agent` tables | `views/Projects.tsx` |
| Fleet | `agent` + `agent_heartbeat` tables | `views/Fleet.tsx` |
| Brain decisions | sched-service event stream | `views/BrainDecisions.tsx` |
| Brain chat | `chat_message` (`chat-relay` module) | `views/BrainChat.tsx` |
| Inference | `inference_request` + `inference_response` + `inference_provider` | `views/Inference.tsx` |
| Architecture health | nexus REST `/api/analyze` | `views/Architecture.tsx` |
| Workplans | `docs/workplans/*.json` via nexus FS API | `views/Workplans.tsx` |
| ADRs | `docs/adrs/*.md` via nexus FS API | `views/Adrs.tsx` |
| Tier routing | `/api/inference/escalation-report` | `views/TierRouting.tsx` |

The exact route table lives in `hex-nexus/assets/src/App.tsx`. Treat the table above as a snapshot — verify against `App.tsx` before quoting.

## Configuration

The dashboard is a static SPA — there is no runtime config beyond the SpacetimeDB host URL it connects to. That URL is injected at build time via the same env that nexus uses (`SPACETIMEDB_HOST`).

Theme + density preferences are stored in `localStorage` per user.

## Security notes (carried over from hexagonal rules)

Primary adapters in the dashboard MUST NOT use `innerHTML` / `outerHTML` / `insertAdjacentHTML` with any data that originates outside the domain layer. Use `textContent` or `createElement`. This is enforced by code review + `hex analyze`.

## Depends On

- **hex-nexus** — serves the SPA, exposes REST endpoints the dashboard calls for non-realtime data.
- **SpacetimeDB** — subscriptions for real-time fleet, inference, brain-decision panels.

## Depended On By

- Nothing — the dashboard is a leaf consumer.

## See also

- `docs/reference/system-architecture.md` — system context.
- `docs/reference/components/hex-nexus.md` — the binary that serves the dashboard.
- `hex-nexus/assets/src/App.tsx` — canonical route table.
- `hex-nexus/CLAUDE.md` — dashboard-specific dev notes (if present).

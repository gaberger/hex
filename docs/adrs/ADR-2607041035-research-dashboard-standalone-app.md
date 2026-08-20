# ADR-2607041035: research-dashboard — a standalone hexagonal app to share results remotely

**Status:** Completed
**Date:** 2026-07-04
**Epoch:** single-agent
**Drivers:** The operator works this box over SSH. Sharing research results (e.g. the DSpark
speculative-decoding test plan) meant either pasting terminal output or asking them to `cat` files
themselves. There was no way to glance at host health (CPU/RAM/disk/GPU) or browse the research docs
from a browser. This ADR is a **backfill** — the app was scaffolded and built directly (see
Consequences) before this ADR was written; it documents the decision retroactively, per the rule that
new ports/adapters/external deps need one.
**Supersedes:**
**Superseded-By:**

## Context

Two needs, one small app: (1) a live system-overview view (CPU/RAM/disk/GPU) for this host, and (2) a
web-servable browser for the markdown research docs under `docs/benchmarks/` and `docs/adrs/` in this
repo, so a remote collaborator can read them in a browser instead of the terminal.

This was explicitly **not** built as a new page inside hex-nexus's existing dashboard (`:5555`,
Solid.js). hex-nexus already has a partial system-stats surface (`/api/resources`, process-level only)
and a reusable ADR-browsing pattern (`ADRBrowser.tsx` + `routes/browse.rs`) — extending those was
considered and rejected by the operator in favor of a **completely new standalone app**, kept separate
from hex-nexus's own architecture and release cycle.

Alternatives considered:
- Extend hex-nexus's dashboard (rejected — operator wants this decoupled from hex-nexus).
- A bare shell script piping `nvidia-smi`/`cat` over SSH port-forwarding (rejected — no browser UI, and
  hex's own no-runtime-scripts rule reserves scripts for build/dev tooling, not user-facing apps).

## Decision

Scaffold a new, standalone hexagonal-architecture app at `examples/research-dashboard/`
(TypeScript + Bun + Express), using the `hex-scaffold` skill's interactive wizard (Minimal MVP style,
no persistence — every request reads live).

- **Domain** (`core/domain/entities.ts`): `SystemSnapshot`, `DocEntry`, `DocContent`. Zero external
  imports.
- **Ports** (`core/ports/index.ts`): `ISystemStatsReader` and `IDocsReader` (driven, implemented by
  secondary adapters), `IDashboardService` (driving, implemented by the use case).
- **Use case** (`core/usecases/service.ts`): `DashboardService`, orchestrates the two driven ports.
- **Secondary adapters**: `system-stats.ts` (shells out to `nvidia-smi`/`df`, reads `node:os` for
  CPU/RAM — no dependency needed for those); `docs-fs.ts` (reads markdown off disk, `collection`-scoped
  base directories, with `safePath()`-style traversal protection consistent with hex's own
  `FileSystemAdapter` convention).
- **Primary adapter**: `http-adapter.ts` (Express ^5 + `marked` for markdown→HTML), serving both an
  HTML UI (`/`, `/system`, `/docs/:collection`, `/docs/:collection/*path`) and a parallel JSON API
  (`/api/system`, `/api/docs/:collection`, `/api/docs/:collection/*path`).
- **Composition root** (`composition-root.ts`): the only file importing from adapters; resolves the
  parent hex repo's `docs/benchmarks` and `docs/adrs` as absolute paths and wires everything together.

New external dependencies: `express ^5.1.0`, `marked ^15.0.6` (adapter-layer only — domain/ports remain
dependency-free, per hex's hexagonal rules).

**Amendment 2026-07-04, same day — "full portal" expansion.** Operator asked for more than the MVP:
richer navigation, live-updating stats, and search, without adding a build step. Chosen approach:
**htmx** (vendored locally as a static asset, `adapters/primary/public/htmx.min.js` — no CDN
dependency, no bundler) layered onto the existing server-rendered Express app, rather than adopting a
client framework (e.g. matching hex-nexus's own Solid.js + Vite stack). Added:
- `IDocsReader.search()` / `IDashboardService.searchDocs()` — new port methods, filename-substring
  search across all collections (kept simple; no content-body indexing).
- A persistent sidebar nav (`adapters/primary/templates.ts`, split out of `http-adapter.ts` to keep
  files under the hex-scaffold size guideline) listing every collection, plus a live-polling home page
  (`hx-trigger="load, every 5s"` against a new `/fragments/system` endpoint) and an as-you-type search
  box (`hx-trigger="keyup changed delay:250ms"` against `/search`).
- Doc collections expanded from 2 (`benchmarks`, `adrs`) to 8: `benchmarks`, `adrs`, `analysis`,
  `specs`, `guides`, `algebra`, `reference`, `examples` — every `docs/*` subdirectory in the parent
  repo that actually contains markdown.

No change to the domain/ports/adapters *boundaries* themselves, only new capability within them — the
architecture from the original decision above holds unchanged.

## Consequences

**Positive:**
- Read-only, stateless, and low-risk: nothing is written or persisted; a request that fails just
  fails, there's no corrupted state to clean up.
- Fully decoupled from hex-nexus — no shared release cycle, no risk of destabilizing the primary
  dashboard while iterating on this.
- Reuses hex's own security convention (path-traversal-safe file serving) rather than inventing a
  weaker one.
- Verified working end-to-end before this ADR was written, not just typechecked: `bun test` (4/4
  pass), `bun run typecheck` (clean), and a live smoke test returned real host data — hostname
  `gary-B650M-C-V3-Y1`, CPU `AMD Ryzen 7 9800X3D`, GPU `RTX 5070 Ti` with live free VRAM, and a correct
  listing of `docs/benchmarks/*.md` including the DSpark test plan.

**Negative:**
- Backfilled, not ADR-first — the code was written before this record, which is exactly the
  "bypassing the loop" pattern hex's own rules warn against. Named directly rather than glossed over.
- A second, independently-versioned app to keep alive (its own `package.json`, its own port) rather
  than reusing hex-nexus's already-running process.
- No auth: anything reachable on the bound port can read host stats and every doc in the two
  collections. Acceptable today only because it's bound to a private `/24` LAN behind existing ufw
  rules — not acceptable if ever exposed beyond that.

**Mitigations:**
- This ADR itself is the mitigation for the backfill gap — recorded now so the decision is visible in
  the ledger, and the pattern isn't repeated silently on the next such app.
- No-auth risk is scoped explicitly in Implementation below (firewall rule limited to the same
  `192.168.30.0/24` subnet already trusted for hex-nexus's own ports) rather than opened broadly.

## Implementation

| Phase | Description | Status | Verification |
|-------|------------|--------|--------------|
| P1 | Scaffold hexagonal skeleton (domain/ports/usecases/adapters/composition-root) via `hex-scaffold` | Done | code:examples/research-dashboard/src |
| P2 | Implement system-stats + docs-fs secondary adapters, http-adapter primary adapter | Done | code:examples/research-dashboard/src/adapters |
| P3 | Unit tests (London-school mocks against the ports) | Done | test:`cd examples/research-dashboard && bun test` |
| P4 | Typecheck + live smoke test | Done | test:`cd examples/research-dashboard && bun run typecheck` |
| P5 | Firewall rule for remote access on the chosen port | Done | `sudo ufw status \| grep 8090` |
| P6 | This ADR (backfill) | Done | code:docs/adrs/ADR-2607041035-research-dashboard-standalone-app.md |
| P7 | htmx (vendored) + sidebar nav + live-polling stats + search | Done | test:`cd examples/research-dashboard && bun test` |
| P8 | Expand doc collections from 2 to all 8 markdown-bearing `docs/*` dirs | Done | code:examples/research-dashboard/src/composition-root.ts |

## References

- `examples/research-dashboard/README.md` — run/test instructions.
- hex-nexus's existing (not reused) patterns: `hex-nexus/src/routes/resources.rs` (process-level
  stats only, no system-wide CPU/RAM/disk/GPU), `hex-nexus/assets/src/components/views/ADRBrowser.tsx`
  + `hex-nexus/src/routes/browse.rs` (the doc-browsing pattern this app deliberately does not extend).
- Security convention followed: `FileSystemAdapter`'s `safePath()` path-traversal protection (CLAUDE.md
  Security section), reimplemented locally in `docs-fs.ts` since this app doesn't depend on hex-nexus.

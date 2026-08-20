# research-dashboard

A small standalone hexagonal-architecture app for sharing research results with a remote
collaborator over a browser instead of the terminal. Capabilities:

- **System overview** (`/`, `/system`) — this host's CPU, memory, disk, and GPU stats, live-polled
  every 5s via htmx (no persistence, no client build step).
- **Docs browser** (`/docs/<collection>`) — renders the markdown research docs from the parent hex
  repo's `docs/{benchmarks,adrs,analysis,specs,guides,algebra,reference,examples}/` directories.
- **Search** — as-you-type, filename-based, across every collection at once.

Interactivity is [htmx](https://htmx.org) (vendored locally, no CDN, no bundler) on top of
server-rendered Express — chosen over a client framework (e.g. hex-nexus's own Solid.js + Vite stack)
to keep this a zero-build-step app. See `docs/adrs/ADR-2607041035-*.md` for the full rationale.

## Architecture

```
src/
  core/
    domain/entities.ts       # SystemSnapshot, DocEntry, DocContent — zero external imports
    ports/index.ts            # ISystemStatsReader, IDocsReader, IDashboardService
    usecases/service.ts       # DashboardService — orchestrates the ports
  adapters/
    primary/
      http-adapter.ts         # Express app: routes only
      templates.ts            # HTML rendering (layout, fragments) — kept separate from routing
      public/htmx.min.js      # vendored, served as a static asset
    secondary/
      system-stats.ts         # shells out to nvidia-smi/df, reads os.* for CPU/RAM
      docs-fs.ts               # reads + searches markdown off disk, with path-traversal protection
  composition-root.ts          # the only file that wires adapters together
```

## Run it

```bash
./start.sh
```

or directly:

```bash
bun install
bun run start          # http://0.0.0.0:8090
```

Override the port/bind address with `PORT` / `HOST` env vars.

## Test

```bash
bun test
bun run typecheck
```

## Remote access

This binds `0.0.0.0` by default. If a firewall is active (this box runs ufw), the chosen port
needs an explicit allow rule for your subnet, e.g.:

```bash
sudo ufw allow from 192.168.30.0/24 to any port 8090 proto tcp
```

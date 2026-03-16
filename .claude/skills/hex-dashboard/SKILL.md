---
name: hex-dashboard
description: Start the hex-intf monitoring dashboard. Use when the user asks to "start dashboard", "open dashboard", "monitor project", "hex-intf dashboard", or "show swarm status".
---

# Hex Dashboard — Project Monitoring

Start the hex-intf dashboard for the **current project directory**.

## Steps

1. Check if a dashboard is already running for THIS project by checking common ports:

```bash
# Check ports 3847-3850 and verify which project is being served
for port in 3847 3848 3849 3850; do
  pid=$(lsof -ti :$port 2>/dev/null | head -1)
  if [ -n "$pid" ]; then
    cmd=$(ps -p $pid -o args= 2>/dev/null)
    echo "Port $port: PID $pid — $cmd"
  fi
done
```

2. If none of those ports serve the current project, find a free port and start:

```bash
hex-intf dashboard --port <free-port> &
```

Use port 3847 if free, otherwise try 3848, 3849, etc.

3. Report the URL to the user: `http://localhost:<port>`

## IMPORTANT

- The `hex-intf hub` command (multi-project) uses port 3847 — if a hub is running, the dashboard should use a DIFFERENT port
- Always verify the process on the port belongs to the current project directory before reporting "already running"
- The dashboard must run from the project's root directory so it reads the correct source files

## What It Shows

- **Architecture Health** — files scanned, violations, dead exports, circular deps
- **Token Efficiency** — AST summary compression ratios (L0-L3)
- **Swarm Status** — active agents, tasks, topology (live from ruflo daemon)
- **Dependency Graph** — interactive import visualization
- **Event Log** — real-time SSE notifications

## Notes

- The dashboard runs as a background process — it stays alive until killed
- SSE stream connects automatically when the page loads
- Swarm data comes from the live ruflo MCP daemon via `mcp exec`
- Kill with: `lsof -ti :<port> | xargs kill`

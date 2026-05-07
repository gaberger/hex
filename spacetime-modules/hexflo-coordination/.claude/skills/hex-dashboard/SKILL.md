---
name: hex-dashboard
description: Start the hex monitoring dashboard. Use when the user asks to "start dashboard", "open dashboard", "monitor project", "hex dashboard", or "show swarm status".
---

# Hex Dashboard — Project Monitoring

Start the hex dashboard for the **current project directory**.

The `hex dashboard` command auto-registers with the project registry at `~/.hex/registry.json` and gets an assigned port (3848-3947). Port 3847 is reserved for the multi-project hub.

## Steps

1. **Check the registry** for an existing registration using the Read tool (NOT bash):

Use the Read tool to read `~/.hex/registry.json`. Look for an entry whose `rootPath` matches the current working directory. If found, note the assigned `port`.

2. **If registered, check if already running** on the assigned port:

```bash
lsof -ti :<assigned-port>
```

If a PID is returned, the dashboard is already running — report the URL `http://localhost:<assigned-port>` and stop.

3. **Start the dashboard** using the Bash tool with `run_in_background: true`:

**CRITICAL**: Do NOT use `&`, `|`, `$(...)`, or any shell operators. Use ONLY the Bash tool's `run_in_background` parameter.

```
Bash(command: "hex dashboard", run_in_background: true)
```

This will:
- Register the project in `~/.hex/registry.json` (if not already)
- Get an assigned port from the registry (3848-3947)
- Write `.hex/project.json` with the project's registry ID
- Start the HTTP server on the assigned port

4. **Wait and verify**:

```bash
sleep 3
```

Then read `.hex/project.json` with the Read tool to confirm the registration.

Report: `http://localhost:<assigned-port>`

## CRITICAL RULES

- **Use Read tool** to read registry — NOT `cat` or `grep` via Bash
- **Use `run_in_background: true`** — NOT the `&` shell operator
- **Do NOT use pipes, command substitution, or shell operators** in any Bash commands
- **Do NOT manually pick ports** — the registry assigns them
- **Do NOT scan ports with lsof in a loop** — read the registry

## What It Shows

- **Architecture Health** — files scanned, violations, dead exports, circular deps
- **Token Efficiency** — AST summary compression ratios (L0-L3)
- **Swarm Status** — active agents, tasks, topology (live from ruflo daemon)
- **Dependency Graph** — interactive import visualization
- **Event Log** — real-time SSE notifications

## Notes

- The dashboard runs as a background process — it stays alive until killed
- SSE stream connects automatically when the page loads
- To stop: find PID with `lsof -ti :<port>` then `kill <PID>`
- Unregister: remove the entry from `~/.hex/registry.json`

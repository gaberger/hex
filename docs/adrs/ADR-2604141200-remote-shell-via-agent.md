# ADR-2604141200: Remote Shell Commands via hex-agent Worker

**Status:** Proposed
**Date:** 2026-04-14
**Drivers:** Users want to run shell commands on remote hosts (bazzite) from their local hex session. Today hex-agent workers poll for tasks but there's no clean way to enqueue a shell command and see the output.

## Context

hex has:
- **hex-agent workers** on remote hosts (bazzite) polling for tasks
- **Inference providers** pointing at bazzite:11434 for local models
- **Brain inbox queue** for async task dispatch

Missing: a way to say "run `nvidia-smi` on bazzite" and get the output back.

## Decision

### 1. `hex hey` shell detection with target host

```bash
hex hey run nvidia-smi on bazzite
hex hey show disk usage on bazzite
hex hey ollama ps on bazzite
```

Classifier detects "on <host>" suffix, routes to remote execution.

### 2. Brain task kind: `remote-shell`

New task kind in brain inbox:
```json
{
  "kind": "remote-shell",
  "payload": {"host": "bazzite", "command": "nvidia-smi"},
  "status": "pending"
}
```

### 3. hex-agent worker polls brain queue on its host

When hex-agent is running on bazzite, it polls `/api/hexflo/memory/search?q=brain-task:` for tasks where `payload.host == <my hostname>`. Executes via whitelisted shell, writes result back.

### 4. Security

- Whitelist: only approved commands (nvidia-smi, df, ollama, ps, systemctl status)
- Host must be pre-registered in `.hex/project.json` trusted_hosts
- Results logged to audit trail

## Implementation

| Phase | Description | Status |
|-------|------------|--------|
| P1 | Add `remote-shell` task kind to brain | Pending |
| P2 | hex-agent worker polls for tasks targeting its hostname | Pending |
| P3 | hex hey classifier detects "on <host>" | Pending |
| P4 | Whitelist + trusted_hosts config | Pending |

## References

- ADR-040: Remote Agent Transport (WebSocket over SSH)
- ADR-2604132330: Brain Inbox Queue
- ADR-2604140000: Hey Hex

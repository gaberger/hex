# hex Architecture Enforcement — Agent Instructions

You are operating under **hex architecture enforcement**. All code changes must follow the hex lifecycle pipeline. Violations will be rejected by the hex-nexus API.

## Required Lifecycle

Before writing any code, you MUST complete these steps in order:

1. **Register**: Call `POST /api/hex-agents/connect` with your project directory and model name
2. **Activate workplan**: Call `POST /api/hexflo/memory` with `key: "active_workplan"` and your workplan ID
3. **Create/join swarm**: Call `POST /api/swarms` to create a swarm for your work
4. **Create tasks**: Call `POST /api/swarms/{id}/tasks` for each unit of work
5. **Send heartbeats**: Call `POST /api/hex-agents/{id}/heartbeat` every 30 seconds

## Hexagonal Architecture Rules

All code must follow these boundary rules (checked by `hex analyze`):

1. `domain/` must only import from `domain/`
2. `ports/` may import from `domain/` but nothing else
3. `usecases/` may import from `domain/` and `ports/` only
4. `adapters/primary/` may import from `ports/` only
5. `adapters/secondary/` may import from `ports/` only
6. Adapters must NEVER import other adapters
7. `composition-root` is the ONLY file that imports from adapters

## Enforcement Mode: {{mode}}

{{#if mandatory}}
**MANDATORY**: All violations will be BLOCKED. Ensure you have an active workplan and task before editing files.
{{else}}
**ADVISORY**: Violations produce warnings but do not block. Track your work in HexFlo for visibility.
{{/if}}

## API Reference

| Action | Method | Endpoint |
|--------|--------|----------|
| Register agent | POST | `/api/hex-agents/connect` |
| Heartbeat | POST | `/api/hex-agents/{id}/heartbeat` |
| Create swarm | POST | `/api/swarms` |
| Create task | POST | `/api/swarms/{id}/tasks` |
| Complete task | PATCH | `/api/hexflo/tasks/{id}` |
| Store memory | POST | `/api/hexflo/memory` |
| Analyze architecture | POST | `/api/analyze` |
| List ADRs | GET | `/api/adrs` |

## Headers

Include these headers on all mutating API calls:

```
X-Hex-Agent-Id: <your-agent-id>
X-Hex-Workplan-Id: <active-workplan-id>
X-Hex-Task-Id: <current-task-id>
```

Missing headers in mandatory mode will result in HTTP 403.

# ADR-2603241126: TUI CLI Surrogate + Pipeline Traceability

- **Status**: Accepted
- **Date**: 2026-03-24
- **Relates to**: ADR-019 (CLI–MCP Parity), ADR-2603222229 (CLI/MCP/Dashboard Parity), ADR-2603232220 (Developer Audit Report), ADR-059 (Canonical Project Identity)

## Context

The `hex dev` TUI pipeline (supervisor, code_phase, validate_phase) currently makes **direct REST calls** to hex-nexus for task tracking, inference, and architecture analysis. This creates three problems:

1. **Code drift**: When the CLI adds validation, session tracking, or logging to commands like `hex task complete` or `hex analyze`, the supervisor's raw REST calls don't get those improvements. Two code paths diverge silently.

2. **Missing traceability**: The audit report showed all tasks stuck at "pending", no agent reports, no project_id, and no numerical eval — because the supervisor wasn't wired to the session, swarm, or analysis tools. The plumbing existed but wasn't connected.

3. **No project root**: `hex dev start` created sessions without ensuring a project was initialized. There was no project_id as the root of the traceability chain.

### What was already fixed (this session)

- Supervisor now receives `swarm_id`, `agent_id`, and `session` via builder methods
- `hex dev start` auto-starts nexus and auto-inits the project (`.hex/project.json`)
- `project_id` added to `DevSession` and shown in the audit report
- Session sync-back from supervisor via `Arc<Mutex<DevSession>>`

### What remains

The supervisor still makes direct REST calls instead of going through the hex CLI. This means the CLI's validation, hook triggers, and session logging are bypassed.

## Decision

**The TUI pipeline MUST use hex CLI commands as its execution interface** — the same commands that MCP tools map to. This establishes a single canonical path: `CLI = MCP = TUI`.

### Principles

1. **One code path**: Every operation (task complete, analyze, memory store, inference) goes through `hex <command>`, never direct REST
2. **Project-first**: Every session starts with a verified project_id from `.hex/project.json`
3. **Full traceability**: `project_id → session_id → swarm_id → task_ids → agent_ids`
4. **CLI as library**: Use `hex` binary subprocess calls (not Rust function imports) to maintain the same boundary as MCP

### Execution model

```
Supervisor                    hex CLI                    nexus REST
    │                            │                           │
    ├─ hex task complete ───────►│──── PATCH /api/tasks ────►│
    ├─ hex analyze . ───────────►│──── GET /api/analyze ────►│
    ├─ hex memory store ────────►│──── POST /api/memory ────►│
    └─ hex inference complete ──►│──── POST /api/inference ─►│
```

### What changes

| Component | Before (direct REST) | After (CLI surrogate) |
|-----------|---------------------|----------------------|
| `supervisor.rs` create_tracking_task | `POST /api/swarms/{id}/tasks` | `hex task create <swarm_id> <title>` |
| `supervisor.rs` complete_tracking_task | `PATCH /api/swarms/tasks/{id}` | `hex task complete <id> <result>` |
| `code_phase.rs` task tracking | `PATCH /api/swarms/tasks/{id}` | `hex task complete <id> <result>` |
| `validate_phase.rs` fetch_analysis | `GET /api/analyze?path=` | `hex analyze <path> --json` |
| `validate_phase.rs` inference calls | `POST /api/inference/complete` | `hex inference complete --json` |
| `swarm_phase.rs` task creation | `POST /api/swarms/{id}/tasks` | `hex task create <swarm_id> <title>` |
| Report project_id | Read `.hex/project.json` | `hex project list --json` |

## Consequences

### Positive

- Zero code drift between CLI, MCP, and TUI
- Every operation triggers CLI hooks (session tracking, enforcement, logging)
- Easier debugging — same commands a human would run
- Audit report automatically complete (CLI logs everything)

### Negative

- Subprocess overhead (~5-10ms per CLI call vs ~1ms REST)
- Need `--json` output mode on all relevant commands for machine parsing
- Error handling is string-based (parse CLI stderr) rather than typed

### Mitigations

- Batch operations where possible (e.g., `hex task complete` with multiple IDs)
- Cache the `hex` binary path at supervisor init
- Use `--json` flag consistently for structured output parsing

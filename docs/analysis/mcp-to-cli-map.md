# MCP Tool to CLI Command Parity Map

**Generated**: 2026-03-22
**Source**: `config/mcp-tools.json` (34 tools) vs `hex-cli/src/main.rs` + `hex-cli/src/commands/mcp.rs`

## Legend

| Status | Meaning |
|--------|---------|
| PARITY | MCP tool has a corresponding CLI command |
| MCP-ONLY | Intentionally MCP-only (no CLI equivalent needed) |
| MCP-DISPATCH-ONLY | Exists in `dispatch_tool()` but not in `mcp-tools.json` |
| CLI-ONLY | CLI command exists but no MCP tool defined |
| GAP | Missing implementation on one side (needs attention) |

## MCP Tools (from `config/mcp-tools.json`)

| MCP Tool | CLI Equivalent | Status | Notes |
|----------|---------------|--------|-------|
| `hex_analyze` | `hex analyze <path>` | PARITY | Falls back to offline analysis if nexus unavailable |
| `hex_analyze_json` | `hex analyze <path>` | PARITY | Same CLI command, MCP variant returns raw JSON; dispatch shares `hex_analyze` handler |
| `hex_status` | `hex status` | PARITY | Both delegate to `/api/version` |
| `hex_hexflo_swarm_init` | `hex swarm init <name>` | PARITY | |
| `hex_hexflo_swarm_status` | `hex swarm status` | PARITY | |
| `hex_hexflo_task_create` | `hex task create <swarm-id> <title>` | PARITY | |
| `hex_hexflo_task_list` | `hex task list` | PARITY | |
| `hex_hexflo_task_assign` | -- | GAP | MCP tool exists + dispatched, but no `hex task assign` CLI subcommand |
| `hex_hexflo_task_complete` | `hex task complete <id> [result]` | PARITY | |
| `hex_hexflo_memory_store` | `hex memory store <key> <value>` | PARITY | |
| `hex_hexflo_memory_retrieve` | `hex memory get <key>` | PARITY | |
| `hex_hexflo_memory_search` | `hex memory search <query>` | PARITY | |
| `hex_adr_list` | `hex adr list` | PARITY | |
| `hex_adr_search` | `hex adr search <query>` | PARITY | |
| `hex_adr_status` | `hex adr status <id>` | PARITY | |
| `hex_adr_abandoned` | `hex adr abandoned` | PARITY | |
| `hex_plan_list` | `hex plan list` | PARITY | MCP reads filesystem directly (LOCAL route) |
| `hex_plan_status` | `hex plan status <file>` | PARITY | MCP reads filesystem directly (LOCAL route) |
| `hex_plan_execute` | `hex plan execute <file>` | PARITY | Delegates to nexus `/api/workplan/execute` |
| `hex_plan_pause` | `hex plan pause` | PARITY | |
| `hex_plan_resume` | `hex plan resume` | PARITY | |
| `hex_plan_report` | `hex plan report <id>` | PARITY | |
| `hex_plan_history` | `hex plan history` | PARITY | |
| `hex_agent_connect` | `hex agent connect` | PARITY | |
| `hex_agent_disconnect` | `hex agent disconnect <id>` | PARITY | |
| `hex_agent_list` | `hex agent list` | PARITY | |
| `hex_nexus_status` | `hex nexus status` | PARITY | |
| `hex_nexus_start` | `hex nexus start` | PARITY | MCP returns guidance text (can't self-start) |
| `hex_secrets_status` | `hex secrets status` | PARITY | |
| `hex_secrets_has` | `hex secrets has <key>` | PARITY | MCP checks env var directly |
| `hex_inbox_notify` | `hex inbox notify` | PARITY | |
| `hex_inbox_query` | `hex inbox list` | PARITY | MCP name differs (`query` vs CLI `list`) |
| `hex_inbox_ack` | `hex inbox ack <id>` | PARITY | |
| `hex_session_start` | -- | MCP-ONLY | Provider-agnostic lifecycle (ADR-2603221959); Claude Code uses hooks instead |
| `hex_session_heartbeat` | -- | MCP-ONLY | Provider-agnostic lifecycle; Claude Code uses `hex hook route` heartbeat |
| `hex_workplan_activate` | -- | MCP-ONLY | Provider-agnostic lifecycle; sets active workplan for enforcement |

## Dispatch-Only Tools (in `dispatch_tool()` but NOT in `mcp-tools.json`)

These tools are handled by the MCP dispatch function but have no entry in the tool definition file, so they will never appear in `tools/list` responses. Clients cannot discover or call them.

| Dispatch Entry | CLI Equivalent | Status | Notes |
|---------------|---------------|--------|-------|
| `hex_inference_add` | `hex inference add` | MCP-DISPATCH-ONLY | Defined in dispatch but missing from `mcp-tools.json` |
| `hex_inference_list` | `hex inference list` | MCP-DISPATCH-ONLY | Defined in dispatch but missing from `mcp-tools.json` |
| `hex_inference_test` | `hex inference test` | MCP-DISPATCH-ONLY | Returns guidance text only |
| `hex_inference_discover` | `hex inference discover` | MCP-DISPATCH-ONLY | Returns guidance text only |
| `hex_inference_remove` | `hex inference remove` | MCP-DISPATCH-ONLY | Defined in dispatch but missing from `mcp-tools.json` |

## CLI-Only Commands (no MCP tool)

These CLI commands have no corresponding MCP tool definition.

| CLI Command | Status | Notes |
|------------|--------|-------|
| `hex init` | CLI-ONLY | Project initialization; interactive, not suited for MCP |
| `hex hook <event>` | CLI-ONLY | Claude Code hook handler; invoked by settings.json, not MCP |
| `hex test <action>` | CLI-ONLY | Integration test runner; developer tool, not agent-facing |
| `hex stdb <action>` | CLI-ONLY | SpacetimeDB management; infrastructure, not agent-facing |
| `hex project <action>` | CLI-ONLY | Project registration; could benefit from MCP exposure |
| `hex readme <action>` | CLI-ONLY | README management; could benefit from MCP exposure |
| `hex skill <action>` | CLI-ONLY | Skill management; informational, low MCP priority |
| `hex enforce <action>` | CLI-ONLY | Enforcement rule management (ADR-2603221959) |
| `hex assets` | CLI-ONLY | Debug command to inspect embedded assets |

## Gaps Requiring Action

### 1. `hex_hexflo_task_assign` -- No CLI equivalent
The MCP tool dispatches correctly to `PATCH /api/hexflo/tasks/{task_id}` with `agent_id`, but there is no `hex task assign <task_id> <agent_id>` CLI subcommand.

**Recommendation**: Add `Assign` variant to `TaskAction` enum in `hex-cli/src/commands/task.rs`.

### 2. Inference tools missing from `mcp-tools.json`
Five inference dispatch handlers exist (`hex_inference_add`, `hex_inference_list`, `hex_inference_test`, `hex_inference_discover`, `hex_inference_remove`) but none are defined in `config/mcp-tools.json`. MCP clients cannot discover these tools.

**Recommendation**: Add tool definitions for at least `hex_inference_add`, `hex_inference_list`, and `hex_inference_remove` to `config/mcp-tools.json`. The `test` and `discover` handlers only return guidance text and may remain CLI-only.

### 3. `hex_analyze_json` -- No separate dispatch handler
The tool is defined in `mcp-tools.json` but has no distinct dispatch case in `dispatch_tool()`. It would fall through to the `Unknown tool` error branch.

**Recommendation**: Either add a dispatch case that calls the same analyze endpoint with a JSON format flag, or remove it from `mcp-tools.json` and document that `hex_analyze` already returns JSON via MCP.

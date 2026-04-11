# CLI-to-MCP Parity Map

**Audit date**: 2026-03-22
**CLI binary**: `./target/release/hex`
**MCP tool source**: `config/mcp-tools.json`

## Legend

| Status | Meaning |
|--------|---------|
| ✓ | MCP tool exists and maps to the CLI command |
| ✗ | No MCP tool exists for this CLI command |
| ~ | Partial coverage (MCP tool exists but differs in scope or naming) |

## Parity Table

| CLI Command | MCP Tool | Status | Notes |
|---|---|---|---|
| `hex analyze [path]` | `hex_analyze` | ✓ | |
| `hex analyze --adr-compliance` | `hex_analyze` | ~ | MCP tool lacks `--adr-compliance` flag; always runs full analysis |
| `hex analyze --strict` | `hex_analyze` | ~ | MCP tool lacks `--strict` flag |
| `hex analyze [path]` (JSON) | `hex_analyze_json` | ✓ | Separate MCP tool for JSON output |
| `hex status` | `hex_status` | ✓ | |
| **Nexus** | | | |
| `hex nexus start` | `hex_nexus_start` | ✓ | |
| `hex nexus stop` | -- | ✗ | No MCP tool to stop the daemon |
| `hex nexus status` | `hex_nexus_status` | ✓ | |
| `hex nexus logs` | -- | ✗ | No MCP tool to tail logs |
| **Agent** | | | |
| `hex agent id` | -- | ✗ | No MCP tool for "who am I?" |
| `hex agent list` | `hex_agent_list` | ✓ | |
| `hex agent info <id>` | -- | ✗ | No MCP tool for single-agent detail |
| `hex agent status <id>` | -- | ✗ | No MCP tool for remote agent status |
| `hex agent connect` | `hex_agent_connect` | ✓ | |
| `hex agent spawn-remote` | -- | ✗ | No MCP tool for remote agent spawning |
| `hex agent disconnect` | `hex_agent_disconnect` | ✓ | |
| `hex agent fleet` | -- | ✗ | No MCP tool for fleet capacity summary |
| `hex agent audit` | -- | ✗ | No MCP tool for commit-vs-task audit |
| **Secrets** | | | |
| `hex secrets has <key>` | `hex_secrets_has` | ✓ | |
| `hex secrets status` | `hex_secrets_status` | ✓ | |
| `hex secrets list` | -- | ✗ | No MCP tool for listing secret grants |
| `hex secrets grant` | -- | ✗ | No MCP tool for creating grants |
| `hex secrets revoke` | -- | ✗ | No MCP tool for revoking grants |
| `hex secrets set` | -- | ✗ | No MCP tool for storing secrets |
| `hex secrets get` | -- | ✗ | No MCP tool for retrieving secrets |
| **SpacetimeDB** | | | |
| `hex stdb status` | -- | ✗ | No MCP tool |
| `hex stdb start` | -- | ✗ | No MCP tool |
| `hex stdb stop` | -- | ✗ | No MCP tool |
| `hex stdb publish` | -- | ✗ | No MCP tool |
| `hex stdb hydrate` | -- | ✗ | No MCP tool |
| `hex stdb generate` | -- | ✗ | No MCP tool |
| **Swarm** | | | |
| `hex swarm init` | `hex_hexflo_swarm_init` | ✓ | |
| `hex swarm status` | `hex_hexflo_swarm_status` | ✓ | |
| `hex swarm list` | -- | ✗ | No MCP tool to list all swarms (status shows active only) |
| **Task** | | | |
| `hex task create` | `hex_hexflo_task_create` | ✓ | |
| `hex task list` | `hex_hexflo_task_list` | ✓ | |
| `hex task complete` | `hex_hexflo_task_complete` | ✓ | |
| -- (no CLI equivalent) | `hex_hexflo_task_assign` | ~ | MCP-only tool; CLI has no `task assign` subcommand |
| **Inbox** | | | |
| `hex inbox list` | `hex_inbox_query` | ✓ | Different name: `list` vs `query` |
| `hex inbox notify` | `hex_inbox_notify` | ✓ | |
| `hex inbox ack` | `hex_inbox_ack` | ✓ | |
| `hex inbox expire` | -- | ✗ | No MCP tool for expiring stale notifications |
| **Memory** | | | |
| `hex memory store` | `hex_hexflo_memory_store` | ✓ | |
| `hex memory get` | `hex_hexflo_memory_retrieve` | ✓ | Different name: `get` vs `retrieve` |
| `hex memory search` | `hex_hexflo_memory_search` | ✓ | |
| **ADR** | | | |
| `hex adr list` | `hex_adr_list` | ✓ | |
| `hex adr status <id>` | `hex_adr_status` | ✓ | |
| `hex adr search <q>` | `hex_adr_search` | ✓ | |
| `hex adr abandoned` | `hex_adr_abandoned` | ✓ | |
| `hex adr review` | -- | ✗ | No MCP tool for ADR consistency review |
| `hex adr schema` | -- | ✗ | No MCP tool for ADR schema/template |
| **Project** | | | |
| `hex project register` | -- | ✗ | No MCP tool |
| `hex project unregister` | -- | ✗ | No MCP tool |
| `hex project archive` | -- | ✗ | No MCP tool |
| `hex project delete` | -- | ✗ | No MCP tool |
| `hex project list` | -- | ✗ | No MCP tool |
| **Plan (Workplan)** | | | |
| `hex plan create` | -- | ✗ | No MCP tool for workplan creation |
| `hex plan list` | `hex_plan_list` | ✓ | |
| `hex plan status <file>` | `hex_plan_status` | ✓ | |
| `hex plan active` | -- | ✗ | No MCP tool for active executions |
| `hex plan history` | `hex_plan_history` | ✓ | |
| `hex plan report <id>` | `hex_plan_report` | ✓ | |
| `hex plan schema` | -- | ✗ | No MCP tool for workplan schema |
| -- (no CLI equivalent) | `hex_plan_execute` | ~ | MCP-only; CLI has no `plan execute` subcommand |
| -- (no CLI equivalent) | `hex_plan_pause` | ~ | MCP-only; CLI has no `plan pause` subcommand |
| -- (no CLI equivalent) | `hex_plan_resume` | ~ | MCP-only; CLI has no `plan resume` subcommand |
| **Inference** | | | |
| `hex inference add` | -- | ✗ | No MCP tool |
| `hex inference list` | -- | ✗ | No MCP tool |
| `hex inference test` | -- | ✗ | No MCP tool |
| `hex inference discover` | -- | ✗ | No MCP tool |
| `hex inference remove` | -- | ✗ | No MCP tool |
| **README** | | | |
| `hex readme sync-adrs` | -- | ✗ | No MCP tool |
| `hex readme interview` | -- | ✗ | No MCP tool |
| **Init** | | | |
| `hex init [path]` | -- | ✗ | No MCP tool |
| **Hook** | | | |
| `hex hook session-start` | -- | ✗ | Hooks are invoked by Claude Code, not by MCP |
| `hex hook session-end` | -- | ✗ | |
| `hex hook pre-edit` | -- | ✗ | |
| `hex hook post-edit` | -- | ✗ | |
| `hex hook pre-bash` | -- | ✗ | |
| `hex hook route` | -- | ✗ | |
| `hex hook pre-agent` | -- | ✗ | |
| `hex hook subagent-start` | -- | ✗ | |
| `hex hook subagent-stop` | -- | ✗ | |
| **MCP** | | | |
| `hex mcp` | -- | -- | This IS the MCP server; not a tool itself |
| **Test** | | | |
| `hex test unit` | -- | ✗ | No MCP tool |
| `hex test arch` | -- | ✗ | No MCP tool |
| `hex test services` | -- | ✗ | No MCP tool |
| `hex test inference` | -- | ✗ | No MCP tool |
| `hex test lint` | -- | ✗ | No MCP tool |
| `hex test all` | -- | ✗ | No MCP tool |
| `hex test e2e` | -- | ✗ | No MCP tool |
| `hex test full` | -- | ✗ | No MCP tool |
| `hex test parity` | -- | ✗ | No MCP tool |
| `hex test history` | -- | ✗ | No MCP tool |
| `hex test trends` | -- | ✗ | No MCP tool |
| **Skill** | | | |
| `hex skill list` | -- | ✗ | No MCP tool |
| `hex skill sync` | -- | ✗ | No MCP tool |
| `hex skill show` | -- | ✗ | No MCP tool |
| **Enforce** | | | |
| `hex enforce list` | -- | ✗ | No MCP tool |
| `hex enforce sync` | -- | ✗ | No MCP tool |
| `hex enforce disable` | -- | ✗ | No MCP tool |
| `hex enforce enable` | -- | ✗ | No MCP tool |
| `hex enforce mode` | -- | ✗ | No MCP tool |
| `hex enforce prompt` | -- | ✗ | No MCP tool |
| **Assets** | | | |
| `hex assets` | -- | ✗ | No MCP tool |
| **Lifecycle (MCP-only)** | | | |
| -- | `hex_session_start` | ~ | MCP-only; replaces `hook session-start` for non-Claude providers |
| -- | `hex_session_heartbeat` | ~ | MCP-only; replaces `hook route` heartbeat for non-Claude providers |
| -- | `hex_workplan_activate` | ~ | MCP-only; no CLI equivalent |

## Summary

| Metric | Count |
|--------|-------|
| Total unique CLI commands (leaf) | 78 |
| CLI commands with MCP tool (✓) | 24 |
| CLI commands with partial MCP (~ ) | 2 |
| CLI commands missing MCP tool (✗) | 51 |
| MCP-only tools (no CLI equivalent) | 6 |
| **Parity rate** | **31%** |

### Categories with full parity
- Memory (3/3)
- Task (3/3, plus 1 MCP-only `task_assign`)
- Inbox (3/4 -- missing `expire`)

### Categories with zero MCP coverage
- SpacetimeDB (0/6)
- Project (0/5)
- Inference (0/5)
- Hook (0/9 -- by design, hooks are CLI-only)
- Test (0/11)
- Skill (0/3)
- Enforce (0/6)
- README (0/2)
- Assets (0/1)
- Init (0/1)

### Notable gaps for agent workflows
1. **`hex agent id`** -- agents cannot discover their own identity via MCP
2. **`hex agent fleet`** -- no fleet capacity visibility via MCP
3. **`hex project list/register`** -- agents cannot manage projects via MCP
4. **`hex plan create`** -- agents cannot create workplans via MCP
5. **`hex plan active`** -- agents cannot check running executions via MCP
6. **`hex secrets grant/revoke/set/get`** -- secret management entirely CLI-only
7. **`hex enforce *`** -- enforcement management entirely CLI-only
8. **`hex inference *`** -- inference provider management entirely CLI-only

# ADR-019: CLI–MCP Parity — Every Command Must Have an MCP Equivalent

## Status: Accepted
## Date: 2026-03-17

## Context

hex has two primary adapters that expose the same use cases through different interfaces:

1. **CLI adapter** (`cli-adapter.ts`) — for human developers in a terminal
2. **MCP adapter** (`mcp-adapter.ts`) — for LLM agents via Model Context Protocol

Both are **driving adapters** in hexagonal architecture. They sit in `adapters/primary/` and call the same port interfaces. This is the entire point of hex architecture — the domain doesn't know or care who is driving it.

However, without an explicit rule, feature development naturally drifts toward one adapter:

- A developer adding a CLI subcommand forgets to add the MCP tool (because they test manually in the terminal)
- An agent-focused feature adds an MCP tool but no CLI equivalent (because the author only tested via Claude)

This drift creates **second-class citizens**. A human can't do what an agent can, or vice versa. In a framework designed for LLM-driven development, this asymmetry is a first-principles violation — hex exists to make human and agent interaction interchangeable.

### Current State (2026-03-17)

| CLI command | MCP equivalent | Notes |
|-------------|---------------|-------|
| `analyze` | `hex_analyze`, `hex_analyze_json` | MCP has extra JSON variant |
| `summarize` | `hex_summarize`, `hex_summarize_project` | MCP has extra project-level variant |
| `validate` | `hex_validate_boundaries` | Parity |
| `scaffold` / `init` | `hex_scaffold` | Parity |
| `generate` | `hex_generate` | Parity |
| `plan` | `hex_plan` | Parity |
| `build` | `hex_build` | Parity |
| `orchestrate` | `hex_orchestrate` | Parity |
| `status` | `hex_status` | Parity |
| `secrets` | `hex_secrets_status`, `hex_secrets_has`, `hex_secrets_resolve` | Parity |
| `adr` | `hex_adr_list`, `hex_adr_search`, `hex_adr_abandoned`, `hex_adr_status` | Parity |
| `dashboard` | `hex_dashboard_start`, `hex_dashboard_register`, `hex_dashboard_unregister`, `hex_dashboard_list`, `hex_dashboard_query` | Parity |
| `hub` | `hex_hub_command`, `hex_hub_command_status`, `hex_hub_commands_list` | Parity |
| `daemon` | — | **Gap**: no MCP equivalent |
| `setup` | — | **Gap**: no MCP equivalent |
| `compare` | — | **Gap**: no MCP equivalent |
| `projects` | — | **Gap**: no MCP equivalent |
| — | `hex_dead_exports` | **Gap**: no CLI equivalent |

## Decision

**Every capability exposed through one primary adapter MUST have an equivalent in the other.** This is a first-principles rule, not a nice-to-have.

### The Parity Principle

1. **Same use case, two skins.** Both adapters call the same port method. The CLI formats for humans (ANSI colors, tables). The MCP adapter formats for agents (structured JSON text).

2. **Add both or add neither.** When implementing a new command:
   - Define the use case method on the appropriate port interface
   - Add the CLI subcommand in `cli-adapter.ts`
   - Add the MCP tool in `mcp-adapter.ts` with matching `hex_` prefix
   - Add the tool to `HEX_TOOLS` registry

3. **Naming convention.** CLI uses kebab-case subcommands (`hex adr list`). MCP uses snake_case with `hex_` prefix (`hex_adr_list`). Subcommands become underscored suffixes.

4. **Argument mapping.** CLI positional args and flags map to MCP `inputSchema` properties:
   ```
   CLI:  hex analyze <path> --json
   MCP:  hex_analyze { path: ".", format: "json" }
   ```

5. **Output contract.** Both adapters return the same semantic content. The CLI may add color/formatting. The MCP adapter wraps results in `{ content: [{ type: "text", text: "..." }] }`. Neither adapter adds logic that doesn't exist in the use case.

### Enforcement

- **CI check**: A test SHALL compare the set of CLI subcommands against `HEX_TOOLS` entries and fail if they diverge. (Implementation tracked separately.)
- **Code review**: PRs adding a command to one adapter without the other MUST be flagged.
- **`hex analyze`**: The architecture health check SHOULD report CLI–MCP parity as a metric.

### Closing the Existing Gaps

| Gap | Action |
|-----|--------|
| `daemon` (CLI only) | Add `hex_daemon_start`, `hex_daemon_stop`, `hex_daemon_status` MCP tools |
| `setup` (CLI only) | Add `hex_setup` MCP tool |
| `compare` (CLI only) | Add `hex_compare` MCP tool |
| `projects` (CLI only) | Add `hex_projects_list`, `hex_projects_add`, `hex_projects_remove` MCP tools |
| `hex_dead_exports` (MCP only) | Add `dead-exports` CLI subcommand (under `analyze` or standalone) |

## Consequences

### Positive

- **Agent–human symmetry**: Any workflow possible in the terminal is equally possible for an LLM agent, and vice versa. This is foundational to hex's value proposition.
- **Use-case-driven design**: Forces features through port interfaces rather than being bolted onto one adapter. This strengthens hexagonal architecture discipline.
- **Testability**: MCP tools are trivially testable (JSON in, JSON out) — parity means every CLI feature gets an easily-automatable test path for free.
- **Discoverability**: `hex_hub_commands_list` and `tools/list` give agents a complete capability inventory. Parity guarantees that inventory is complete.

### Negative

- **Higher per-feature cost**: Every new command requires work in two files instead of one (~20-30 extra lines per command).
- **Naming discipline**: Teams must agree on the CLI ↔ MCP name mapping for each new feature.
- **Existing gap closure**: 5 gaps need backfilling (estimated: small effort per gap since use cases already exist).

## Alternatives Considered

1. **CLI-only with MCP auto-generation** — Generate MCP tools from CLI metadata. Rejected: CLI and MCP have different argument shapes (positional vs named), different output needs (ANSI vs JSON), and auto-generation produces poor tool descriptions that confuse agents.

2. **MCP-only, deprecate CLI** — Let agents do everything. Rejected: humans need terminal workflows for debugging, CI pipelines, and environments without MCP support.

3. **Soft guideline instead of hard rule** — "Try to add both." Rejected: soft guidelines erode. The existing 5 gaps prove drift happens naturally without enforcement.

4. **Single adapter with format flag** — One adapter that switches output format. Rejected: violates hexagonal architecture (one adapter, two concerns). CLI needs process exit codes, interactive prompts, and streaming output that don't map to MCP's request/response model.

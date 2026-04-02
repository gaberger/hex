# Hooks Investigation Audit: hex Features vs Claude Code Hooks

**Date**: 2026-04-01
**Purpose**: Classify every hex feature by whether it belongs in Claude Code hooks, SpacetimeDB (STDB), both, or neither — to guide the hooks migration effort.

## Classification Scheme

| Label | Meaning |
|-------|---------|
| **HOOKS** | Maps directly to a Claude Code hook event (PreToolUse/PostToolUse/UserPromptSubmit/SubagentStart/SubagentStop/SessionStart/SessionEnd). Operates locally on a single agent session. |
| **STDB** | Requires cross-host visibility, real-time WebSocket push to multiple subscribers, or distributed coordination (CAS locks, optimistic concurrency, multi-agent task claiming). |
| **BOTH** | Needs local enforcement via hook AND persistent cross-session/cross-host state in STDB. The hook triggers the check; STDB holds the authoritative state. |
| **NEITHER** | Pure local computation with no hook or coordination need, or effectively dead/redundant infrastructure. |

---

## Hook YAML Definitions (`hex-cli/assets/hooks/hex/`)

| Feature | Current Location | Classification | Rationale |
|---------|-----------------|----------------|-----------|
| Boundary check on file edits | `hex-boundary-check.yml` (PreToolUse Write/Edit/MultiEdit) | BOTH | Hook triggers `hex analyze` tree-sitter check locally; violations are logged to STDB for cross-session visibility |
| Lifecycle enforcement (workplan+swarm active) | `hex-lifecycle-enforcement.yml` (PreToolUse Write/Edit) | BOTH | Hook enforces gate locally by calling nexus REST; workplan/swarm state lives in STDB |
| Architecture gate on tool use | `hex-architecture-gate.yml` (PreToolUse) | HOOKS | Local structural check delegating to `hex analyze`; no cross-host state needed |
| Specs-required gate | `hex-specs-required.yml` (UserPromptSubmit) | HOOKS | Local intent classification; blocks work-like prompts if no spec exists in repo |
| Merge validation | `hex-merge-validation.yml` (SubagentStop) | BOTH | Hook triggers auto-merge + worktree cleanup locally; merge result recorded in STDB |
| No REST state mutation | `hex-no-rest-state-mutation.yml` (PreToolUse Bash) | HOOKS | Local pattern match on bash commands; no STDB state needed |
| ADR lifecycle enforcement | `hex-adr-lifecycle.yml` (PreToolUse Write) | HOOKS | Local file-pattern check; ensures ADR exists before implementation files |

---

## Hook Events (`hex-cli/src/commands/hook.rs`)

| Hook Event | Claude Code Trigger | Classification | What It Does |
|-----------|---------------------|----------------|--------------|
| `route` | UserPromptSubmit | BOTH | Heartbeat to nexus + inbox check + fingerprint staleness refresh + intent classification + workplan enforcement on work-like prompts |
| `pre_edit` | PreToolUse Write/Edit/MultiEdit | BOTH | Boundary validation (delegates to `hex analyze`) + lifecycle enforcement (workplan+swarm) + adapter boundary (allowed_paths check) |
| `post_edit` | PostToolUse Write/Edit | BOTH | Nexus notification + edit counter + HexFlo memory update + README/ADR sync |
| `pre_bash` | PreToolUse Bash | HOOKS | Destructive command detection (blocks `rm -rf` patterns) outside SHIP phase |
| `pre_agent` | PreToolUse Agent | BOTH | HEXFLO_TASK enforcement + workplan+swarm check for background agents |
| `subagent_start` | SubagentStart | BOTH | Worktree isolation enforcement + task assignment via nexus REST + tier gate check |
| `subagent_stop` | SubagentStop | BOTH | Task completion via nexus REST + auto-merge worktree branch + worktree cleanup |
| `session_start` | SessionStart | BOTH | Agent registration with nexus + fingerprint injection to stdout + workplan context load |
| `session_end` | SessionEnd | BOTH | Session progress flush to HexFlo memory + agent deregistration |

---

## hex-agent Secondary Adapters (`hex-agent/src/adapters/secondary/`)

| Adapter | Classification | Rationale |
|---------|----------------|-----------|
| `permission.rs` | HOOKS | Local tool permission enforcement; session-scoped |
| `sandboxed_fs.rs` | HOOKS | Local filesystem path traversal protection |
| `output_analyzer.rs` | BOTH | Analyzes agent output locally; scores written to STDB via RL engine |
| `rl_client.rs` | STDB | Sends scores to RL engine WASM module; requires STDB |
| `context_manager.rs` | BOTH | Local context assembly + reads fingerprint from STDB |
| `token_metrics.rs` | BOTH | Local token counting; totals reported to STDB inference-gateway |
| `haiku_preflight.rs` | HOOKS | Local pre-flight check for Haiku model tier; no STDB state |
| `spacetime_coordination.rs` | STDB | Direct STDB reducer calls for swarm/task coordination |
| `hub_client.rs` | BOTH | REST client to hex-nexus; bridges local agent to STDB-backed state |
| `swarm_spawner.rs` | BOTH | Spawns background agents; registers tasks in STDB via nexus |
| `stdb_task_poller.rs` | STDB | Polls STDB for available tasks; genuinely multi-host |
| `claude_code_inference.rs` | HOOKS | Local Claude Code subprocess invocation; session-scoped |
| `live_context.rs` | BOTH | Loads live context from filesystem + STDB fingerprint cache |
| `spacetime_skill.rs` | STDB | Reads skill definitions from STDB skill-registry module |
| `spacetime_hook.rs` | STDB | Reads hook definitions from STDB hook-registry module |
| `spacetime_agent.rs` | STDB | Agent registration/heartbeat/deregistration in STDB agent-registry |
| `prompt.rs` | HOOKS | Local prompt assembly; no STDB state |
| `env_secrets.rs` | HOOKS | Local environment variable secret loading |
| `hub_claim_secrets.rs` | STDB | Claims secrets from STDB secret-grant module |
| `rate_limiter.rs` | HOOKS | Local in-process rate limiting; session-scoped |
| `mcp_config.rs` | HOOKS | Local MCP config file reading |
| `mcp_stdio_client.rs` | HOOKS | Local stdio MCP client; session-scoped |
| `skill_loader.rs` | BOTH | Loads skills from embedded assets + syncs with STDB skill-registry |
| `agent_loader.rs` | BOTH | Loads agent YAMLs from embedded assets + registers with STDB |
| `nexus_inference.rs` | STDB | Routes inference requests through nexus to STDB inference-gateway |
| `stdb_inference.rs` | STDB | Direct STDB inference-gateway reducer calls |
| `openai_compat.rs` | NEITHER | OpenAI-compatible shim; adapter for external provider, no hex coordination |
| `command_session.rs` | BOTH | Manages command session lifecycle; state tracked in STDB |
| `controller_worker.rs` | STDB | Polls STDB for controller tasks; genuinely multi-host |
| `code_phase_worker.rs` | BOTH | Executes code phases locally; reports progress to STDB |
| `task_executor.rs` | BOTH | Executes tasks locally; updates task status in STDB |
| `stdb_connection.rs` | STDB | STDB WebSocket connection management |
| `inference_client.rs` | BOTH | Local inference routing with STDB fallback |
| `spacetime_inference.rs` | STDB | Inference state in STDB; genuinely multi-host |
| `tools.rs` | HOOKS | Local tool permission definitions |
| `anthropic.rs` | HOOKS | Direct Anthropic API client; local session |

---

## Domain Modules (`hex-agent/src/domain/`)

| Module | Classification | Rationale |
|--------|----------------|-----------|
| `hex_knowledge.rs` | NEITHER | Pure domain logic; architecture knowledge base, zero external deps |
| `output_score.rs` | NEITHER | Pure value object; score calculation logic, no I/O |
| `context.rs` | NEITHER | Pure domain type; context assembly data structures |
| `skills.rs` | NEITHER | Pure domain type; skill definition structs |
| `pricing.rs` | NEITHER | Pure domain logic; model pricing calculations |
| `mcp.rs` | NEITHER | Pure domain type; MCP tool definition structs |

---

## SpacetimeDB WASM Modules (`spacetime-modules/`)

| Module | Classification | Rationale |
|--------|----------------|-----------|
| `hexflo-coordination` | STDB | Core: swarms, tasks, agents, memory, projects, config — genuinely multi-host |
| `agent-registry` | STDB | Agent lifecycle + heartbeats — cross-host visibility required |
| `architecture-enforcer` | BOTH | Server-side boundary validation triggered by hooks; STDB for cross-session results |
| `fleet-state` | STDB | Compute node registry — genuinely multi-host |
| `inference-gateway` | STDB | LLM request routing — multi-host coordination |
| `workplan-state` | STDB | Task status + phase tracking — cross-agent coordination |
| `skill-registry` | STDB | Skill definitions — shared across all agents |
| `chat-relay` | STDB | Message routing — multi-host by nature |
| `rl-engine` | STDB | Reinforcement learning feedback loop — cross-session state |
| `file-lock-manager` | STDB | CAS file locking — genuinely requires distributed coordination |
| `secret-grant` | STDB | Secret grants — cross-host access control |
| `conflict-resolver` | STDB | Conflict resolution — cross-agent coordination |
| `neural-lab` | STDB | Neural Lab experiments — cross-session state |
| `hexflo-lifecycle` | STDB | Swarm lifecycle events — cross-host |
| `hook-registry` | STDB | Hook definitions — shared across all agents |
| `test-results` | STDB | Test result storage — cross-session reporting |
| `agent-definition-registry` | STDB | Agent YAML definitions — shared across agents |
| `inference-bridge` | STDB | Model integration bridge — multi-host |

---

## CLI Commands (`hex-cli/src/commands/`)

| Command Group | Classification | Rationale |
|---------------|----------------|-----------|
| `hook` (route/pre_edit/post_edit/pre_bash/pre_agent/subagent_start/subagent_stop/session_start/session_end) | BOTH | Hook events with STDB state backing |
| `swarm` (init/status/create/complete) | STDB | Swarm coordination state in STDB |
| `task` (create/list/complete/assign) | STDB | Task state in STDB |
| `memory` (store/get/search) | STDB | Persistent memory in STDB |
| `analyze` | HOOKS | Local tree-sitter analysis; no STDB state |
| `enforce` (list/mode/prompt/sync) | BOTH | Local enforcement rules + STDB-backed config sync |
| `adr` (list/status/search/abandoned) | NEITHER | Local filesystem ADR operations |
| `inbox` (list/notify/ack) | STDB | Notification inbox in STDB |
| `status` | BOTH | Aggregates local + STDB state |
| `nexus` (start/status) | BOTH | Controls nexus daemon; nexus bridges to STDB |
| `secrets` (status/has/grant/revoke) | STDB | Secret grants in STDB secret-grant module |
| `project` (list/register) | STDB | Project registry in STDB |
| `report` | BOTH | Local cost reporting aggregated from STDB inference records |

---

## Summary

| Classification | Count | Key Examples |
|----------------|-------|-------------|
| **HOOKS only** | ~12 | pre_bash, specs-required gate, architecture-gate YAML, sandboxed_fs, permission, rate_limiter, env_secrets, anthropic, tools, prompt, mcp_config, haiku_preflight |
| **STDB only** | ~16 | All WASM modules, stdb_task_poller, controller_worker, spacetime_coordination, stdb_connection, swarm/task/memory CLI commands |
| **BOTH** | ~18 | route, pre_edit, post_edit, pre_agent, subagent_start/stop, session_start/end, hub_client, output_analyzer, context_manager, lifecycle-enforcement YAML, enforce CLI, status CLI |
| **NEITHER** | ~7 | All domain modules (hex_knowledge, output_score, context, skills, pricing, mcp), openai_compat |

### Key Finding

The majority of hex's operational features are **BOTH**: they need a Claude Code hook event as a trigger (to intercept the right moment in the development workflow) AND SpacetimeDB for persistent, cross-session, cross-host state. Pure HOOKS features are those that operate entirely within a single agent session with no coordination state. Pure STDB features are those driven by distributed polling (workers claiming tasks) rather than event-driven hook triggers.

The `validate_boundary_edit` function in hook.rs is mostly a stub — real boundary checking is done by `hex analyze` (tree-sitter via nexus REST). This confirms that HOOKS classifications for structural checks are accurate: the hook triggers the check but delegates heavy lifting to the CLI/nexus.

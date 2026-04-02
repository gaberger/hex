# Risk Assessment: Hooks Migration (hex-agent + hex-nexus → Claude Code Hooks + Minimal SpacetimeDB)

**Date:** 2026-04-01  
**Author:** hexflo-investigator (task 35f7b496)  
**Scope:** Evaluate replacing the hex-agent Rust daemon and hex-nexus orchestration daemon with Claude Code lifecycle hooks plus a minimal SpacetimeDB module for state that genuinely requires persistence.

---

## 1. What Breaks Immediately

If `hex-agent` and `hex-nexus` binaries were removed today, the following stop working entirely because they make HTTP calls to `NexusClient` (port 5555) or depend on the running hex-agent daemon.

### 1.1 hex CLI Commands — Nexus-Dependent (immediate breakage)

| Command | File | Dependency |
|---|---|---|
| `hex nexus start/stop/status/logs` | `hex-cli/src/commands/nexus.rs` | Spawns/manages the `hex-nexus` binary directly |
| `hex agent list/info/status/connect/spawn-remote/disconnect/fleet/worker` | `hex-cli/src/commands/agent.rs` | All subcommands call `NexusClient` REST API |
| `hex agent audit / worktree-audit / evict` | `hex-cli/src/commands/agent.rs` | Same — REST to hex-nexus |
| `hex swarm init/status/list/complete/fail/cleanup` | `hex-cli/src/commands/swarm.rs` | All call `NexusClient` HexFlo endpoints |
| `hex secrets list/grant/revoke/set/get` | `hex-cli/src/commands/secrets.rs` | REST to hex-nexus `/api/secrets/*`; only `has`/`status` survive locally |
| `hex inference add/list/test/discover/remove/setup/watch` | `hex-cli/src/commands/inference.rs` | REST to hex-nexus inference routes |
| `hex enforce list/sync/disable/enable/mode` | `hex-cli/src/commands/enforce.rs` | Calls nexus for SpacetimeDB rules; `prompt` subcommand survives locally |
| `hex plan create/list/status/active/history/report/reconcile` | `hex-cli/src/commands/plan.rs` | REST to hex-nexus workplan executor |
| `hex task assign/complete/list` | `hex-cli/src/commands/task.rs` | REST to hex-nexus HexFlo task routes |
| `hex inbox notify/query/ack` | `hex-cli/src/commands/inbox.rs` | REST to hex-nexus notification system |
| `hex status` | `hex-cli/src/commands/status.rs` | Aggregates nexus status; degrades to local-only |

### 1.2 MCP Tools — Nexus-Dependent (immediate breakage)

All MCP tools in `main.go` forward to `hex-nexus` REST endpoints. The following tools break entirely:

- `hex_agent_list`, `hex_agent_info`, `hex_agent_connect`, `hex_agent_disconnect`, `hex_agent_id`
- `hex_hexflo_swarm_init`, `hex_hexflo_swarm_status`
- `hex_hexflo_task_create`, `hex_hexflo_task_assign`, `hex_hexflo_task_complete`, `hex_hexflo_task_list`
- `hex_hexflo_memory_store`, `hex_hexflo_memory_retrieve`, `hex_hexflo_memory_search`
- `hex_inference_add`, `hex_inference_list`, `hex_inference_test`, `hex_inference_discover`, `hex_inference_remove`
- `hex_secrets_grant`, `hex_secrets_revoke`, `hex_secrets_vault_set`, `hex_secrets_vault_get`, `hex_secrets_status`
- `hex_enforce_list`, `hex_enforce_sync`, `hex_enforce_mode`, `hex_enforce_prompt`
- `hex_plan_execute`, `hex_plan_list`, `hex_plan_status`, `hex_plan_pause`, `hex_plan_resume`, `hex_plan_report`
- `hex_nexus_start`, `hex_nexus_status`
- `hex_inbox_notify`, `hex_inbox_query`, `hex_inbox_ack`

### 1.3 Commands That Survive (local-only)

- `hex analyze` — calls `hex-nexus` treesitter analyzer but degrades to local rust binary if needed
- `hex enforce prompt` — reads `.hex/adr-rules.toml` locally, no daemon needed
- `hex secrets has/status` — reads env vars only
- `hex adr *`, `hex spec *`, `hex git *`, `hex fingerprint`, `hex readme`, `hex report` — file-system only

---

## 2. Migration Complexity by Capability

### 2.1 Architecture Enforcement

**Current:** `hex-nexus/src/orchestration/constraint_enforcer.rs` (163 lines) validates `AgentConstraints` (forbidden_paths, hex_layer, max_file_size, allow_bash, allow_write) fetched from SpacetimeDB `agent_definition` table at agent spawn time. Rules are also synced to SpacetimeDB from `.hex/adr-rules.toml` via `hex enforce sync`.

**Lines of code involved:** ~163 (ConstraintEnforcer) + ~702 (config_sync.rs) + ~509 (analysis/analyzer.rs) = ~1,374 lines in hot path

**External dependencies to keep:** SpacetimeDB `rl_engine` module and `hook_registry` table (hooks fetch their configs from here). The `hex analyze` pipeline via treesitter must remain.

**Hooks equivalent complexity:** Low-to-medium.
- A `PreToolUse` hook script (~50 lines shell/Python) reading `.hex/adr-rules.toml` can block `Write`/`Edit` to forbidden paths and wrong hex layers.
- `hex analyze` already runs as a standalone binary; a `PostToolUse` hook calling it after every Write is ~20 lines.
- Gap: hooks cannot inspect the *agent definition* (the role-level constraint that says "this agent may only touch `src/adapters/primary`") without a running registry. A minimal SQLite file at `.hex/agent-constraints.json` (written once at `hex init`) would substitute SpacetimeDB for local single-machine use.

**Risk:** Low for local single-machine enforcement. Medium if remote multi-agent enforcement is required.

---

### 2.2 Permissions / Secret Distribution

**Current:** `hex-nexus/src/adapters/spacetime_secrets.rs` (760 lines) + `hex-nexus/src/routes/secrets.rs` (519 lines). Implements time-limited grants stored in SpacetimeDB. Agents claim keys via `hub_claim_secrets.rs` adapter in hex-agent.

**Lines of code involved:** ~1,279 lines in the secrets subsystem

**External dependencies to keep:** SpacetimeDB as a secret grant store (or replace with a local SQLite grant table). The TLS channel between hex-nexus and remote agents for key distribution cannot be replaced by hooks.

**Hooks equivalent complexity:** High for multi-agent scenarios; Low for solo developer use.
- For single-machine: a `PreToolUse` hook can inject `ANTHROPIC_API_KEY` from keychain or `.env`. ~30 lines.
- For multi-machine: the grant/revoke/TTL lifecycle fundamentally requires a network service. Hooks cannot distribute secrets to remote agents. This is an **irreducible nexus dependency** in remote swarm mode.

**Risk:** High if remote agents are used. Low for local use.

---

### 2.3 Cost Tracking / Token Budget

**Current:** `hex-agent/src/adapters/secondary/stdb_inference.rs` (503 lines). Routes LLM calls through SpacetimeDB `inference_gateway` module. No REST fallback — inference fails immediately if SpacetimeDB is unavailable. The `hex-agent/src/ports/rl.rs` (363 lines) defines the RL port for model selection and token budget management.

**Lines of code involved:** ~503 (stdb inference adapter) + ~363 (rl port) + ~1,352 (hex-nexus routes/inference.rs) = ~2,218 lines

**External dependencies to keep:** If cost tracking against a shared budget is needed, some persistent store is required. `stdb_inference.rs` has **no REST fallback by design** (the code comment explicitly states this). Removing SpacetimeDB kills all inference routing for hex-agent sessions that use the `spacetimedb` feature flag.

**Hooks equivalent complexity:** Medium.
- A `PostToolUse` hook can append token usage to a local JSONL file per-session. ~40 lines.
- Model selection logic (RL-based routing to Ollama vs cloud) requires a full rewrite or retention of a minimal SpacetimeDB inference-gateway module.
- The `inference watch` command (long-running daemon that dispatches queued inference tasks) has no hooks equivalent — it requires a polling process.

**Risk:** High. This is the strongest argument to keep a minimal SpacetimeDB module.

---

### 2.4 Architecture Enforcement (hex analyze / ADR rules)

**Current:** `hex-nexus/src/analysis/analyzer.rs` (509 lines), `treesitter_adapter.rs` (919 lines), `fingerprint_extractor.rs` (661 lines). Hex analyze runs as a route on the nexus HTTP server.

**Lines of code involved:** ~2,089 lines in the analysis subsystem

**External dependencies to keep:** Tree-sitter grammars (already compiled into hex-nexus). The `hex-parser` Rust crate is standalone and does not require the daemon.

**Hooks equivalent complexity:** Low. `hex analyze` can be invoked as a subprocess from a `PostToolUse` or `Stop` hook. The analysis binary (`hex analyze`) already exists as a CLI command. A hook wrapper is ~15 lines.

**Risk:** Low. This is a clean candidate for hooks replacement.

---

### 2.5 Swarm Coordination

**Current:** `hex-nexus/src/coordination/mod.rs` (573 lines) + `routes/swarms.rs` (528 lines) + `adapters/spacetime_coordination.rs` in hex-agent (451 lines) + SpacetimeDB `fleet-state` module. Swarm tasks are distributed via the HexFlo task queue, agents poll for tasks, progress is tracked in SpacetimeDB tables (`agent_registry`, `agent_heartbeat`, `compute_node`).

**Lines of code involved:** ~1,552 lines in swarm coordination (not counting SpacetimeDB bindings ~2,000+ lines)

**External dependencies to keep:** SpacetimeDB `fleet-state` and `agent_definition_registry` modules are the source of truth for which agents are alive and which tasks are assigned. This is inherently a distributed state problem — hooks cannot solve it.

**Hooks equivalent complexity:** Not applicable for multi-agent coordination. A single-agent workflow can be approximated with file-based task queues (`.hex/tasks/`) and hooks that read them. For actual swarm use (multiple Claude Code instances coordinating), a network service is irreplaceable.

**Risk:** High. Swarm coordination is the capability most fundamentally incompatible with a hooks-only architecture.

---

### 2.6 Inference Routing

**Current:** `hex-nexus/src/adapters/inference_router.rs` (142 lines) + `routes/inference.rs` (1,352 lines). Routes inference requests to best available agent by model availability, load, and locality. Uses SpacetimeDB `remote_registry` for live agent discovery.

**Lines of code involved:** ~1,494 lines

**External dependencies to keep:** SpacetimeDB `agent_registry` for live endpoint discovery. The Ollama/vLLM provider adapters in hex-nexus must remain if local model routing is needed.

**Hooks equivalent complexity:** Medium. A `PreToolUse` hook can override the model by writing to a config file that hex-agent reads at session start. But dynamic routing to Ollama endpoints discovered at runtime cannot be done purely in hooks.

**Risk:** Medium. Simple provider registration works via hooks; dynamic load-balanced routing requires the daemon.

---

## 3. What Can Be Deleted

If hooks handle local single-machine arch enforcement and hooks replace the hex-agent daemon for solo workflows, the following become dead code:

### hex-nexus modules (safe to delete)
- `hex-nexus/src/orchestration/constraint_enforcer.rs` — replaced by PreToolUse hook
- `hex-nexus/src/orchestration/workplan_executor.rs` — only needed for autonomous multi-agent execution
- `hex-nexus/src/remote/deployer.rs` — remote deployment only
- `hex-nexus/src/remote/ssh.rs` — remote agent SSH transport
- `hex-nexus/src/adapters/ssh_tunnel.rs` — SSH tunnel management
- `hex-nexus/src/adapters/remote_registry.rs` — remote agent discovery
- `hex-nexus/src/adapters/agent_lifecycle.rs` — remote agent lifecycle management
- `hex-nexus/src/adapters/docker_sandbox.rs` — sandbox isolation

### hex-agent adapters (safe to delete if no SpacetimeDB)
- `hex-agent/src/adapters/secondary/spacetime_hook.rs` (581 lines) — replaced by local hook runner
- `hex-agent/src/adapters/secondary/stdb_inference.rs` (503 lines) — replaced by direct Anthropic/Ollama calls
- `hex-agent/src/adapters/secondary/spacetime_coordination.rs` (451 lines) — swarm coordination gone
- `hex-agent/src/adapters/secondary/spacetime_agent.rs` (331 lines) — agent registry gone
- `hex-agent/src/adapters/secondary/haiku_preflight.rs` — cost-optimization preflight (nexus-only)
- `hex-agent/src/adapters/secondary/hub_claim_secrets.rs` — secrets from nexus gone

### SpacetimeDB modules (safe to delete if hooks replace)
- `spacetime-modules/fleet-state/` — fleet coordination module
- The `hook_registry` SpacetimeDB module (hooks are now in `settings.json`, not SpacetimeDB)

**Estimated LOC that becomes dead code: ~4,500–5,500 lines** across hex-agent secondary adapters + hex-nexus remote/orchestration subsystems.

---

## 4. Rollback Path

If hooks-based enforcement proves insufficient, the revert path is:

### Scenario A: Hooks can't block a tool type

**Problem:** Claude Code PreToolUse hooks with `type: command` can only block tool calls that execute external commands. Pure in-process tool executions (e.g. internal MCP tool calls not going through subprocess) may not be interceptable.

**Evidence from codebase:** `hook_dispatch.rs` shows the existing hook system already handles `blocking` hooks by checking `r.success()`. The existing `HookRunnerPort` pattern is sound — it's just currently backed by SpacetimeDB rather than `settings.json`.

**Revert:** Re-enable `hex-nexus` daemon (`hex nexus start`). The `NexusClient` in all CLI commands already handles graceful degradation — `enforce list` falls back to local rules if nexus is unreachable. Full daemon restoration requires only `cargo build -p hex-nexus && hex nexus start`.

### Scenario B: Hooks performance is too slow

**Problem:** A shell-spawned hook for every `PreToolUse` event adds latency proportional to shell startup time (~20–100ms per tool call). Under heavy swarm load this compounds.

**Revert:** Move enforcement back into the hex-agent Rust daemon (the `HookDispatcher` + `HookRunnerPort` already exists in hex-agent). No architectural change needed — only swap the settings-file-backed hook runner for the SpacetimeDB-backed one.

### Scenario C: Multi-agent coordination breaks

**Problem:** Without a running nexus/SpacetimeDB, multiple Claude Code instances cannot coordinate task assignment. File-based locking is fragile under parallel writes.

**Revert:** `hex nexus start` restores full swarm coordination. The SpacetimeDB bindings in hex-nexus are complete (37,742 LOC) and the fleet-state module is battle-tested.

### Scenario D: Secrets distribution to remote agents fails

**Revert:** Re-enable `hex nexus start` with `hex agent connect <nexus-url>` on remote machines. No code changes needed.

**Key point:** The rollback is a single binary restart in all scenarios because hex-nexus is still compiled and present in the repo. The migration to hooks is an operational change, not a deletion of code.

---

## 5. Go/No-Go Recommendation

### Recommendation: CONDITIONAL GO — proceed with hooks for local arch enforcement only; retain minimal SpacetimeDB for inference routing and swarm state.

### Rationale

**Strong case for hooks (GO):**
1. Architecture enforcement (forbidden paths, hex layer boundaries) is a natural PreToolUse hook. It runs fast, requires no daemon, and is already modeled in `adr-rules.toml`. The `ConstraintEnforcer` (163 lines) becomes a ~50-line shell script.
2. `hex analyze` invocation post-edit is a clean PostToolUse hook (~15 lines calling the existing binary).
3. Hook infrastructure already exists in hex-agent — `HookDispatcher`, `HookRunnerPort`, `HookConfig`, `HookEvent` are fully implemented. The `spacetime_hook.rs` adapter already has a REST fallback mode for local use without SpacetimeDB.
4. ADR-002 / ADR-018 enforcement (treesitter layer checks) works perfectly as a Stop hook that fails the session if violations are found.

**Strong case against full daemon removal (NO-GO for complete removal):**
1. `stdb_inference.rs` has **no fallback** — comment explicitly states "inference fails immediately if SpacetimeDB unavailable." Removing SpacetimeDB kills multi-provider inference routing. This is ~503 lines with no hooks equivalent.
2. Swarm coordination (`hex swarm init`, `hex agent worker`, `hex task assign`) is architecturally incompatible with a stateless hooks model. 30+ MCP tools would be broken permanently.
3. Secrets distribution to remote agents is a hard network dependency. Hooks cannot replace a TTL-based grant store for multi-machine workflows.
4. The `inference watch` daemon (continuous polling + dispatching) has no hooks analog.

### Minimum Viable Hooks Setup (equivalent safety for local solo use)

**Three hooks in `settings.json` provide ~80% of current enforcement value:**

```json
{
  "hooks": {
    "PreToolUse": [
      {
        "matcher": "Write|Edit|MultiEdit",
        "hooks": [{
          "type": "command",
          "command": "hex enforce check-file $TOOL_INPUT_PATH",
          "blocking": true,
          "timeout": 5000
        }]
      }
    ],
    "PostToolUse": [
      {
        "matcher": "Write|Edit|MultiEdit",
        "hooks": [{
          "type": "command",
          "command": "hex analyze --file $TOOL_INPUT_PATH --quiet",
          "blocking": false,
          "timeout": 10000
        }]
      }
    ],
    "Stop": [
      {
        "hooks": [{
          "type": "command",
          "command": "hex analyze --violations-only --exit-code",
          "blocking": true,
          "timeout": 30000
        }]
      }
    ]
  }
}
```

**What this gives you:**
- Pre-write path validation against `adr-rules.toml` (forbidden_paths, hex_layer)
- Post-write layer boundary checking via treesitter analysis
- Session-exit gate that blocks completion if arch violations exist

**What still requires hex-nexus:**
- All multi-agent coordination (`hex swarm`, `hex agent worker`)
- Multi-provider inference routing (Ollama, vLLM, OpenRouter)
- Secret grant distribution to remote agents
- Cost tracking and RL-based model selection
- The full `hex plan execute` autonomous execution pipeline

### Migration Sequencing (if proceeding)

1. **Phase 1 (low risk, 1 week):** Implement the three hooks above. Validate that `hex enforce check-file` can be a thin CLI wrapper reading `.hex/adr-rules.toml` without daemon. This replaces ConstraintEnforcer entirely for local use.

2. **Phase 2 (medium risk, 2 weeks):** Extract inference provider config to `.hex/providers.toml`. Allow `hex inference add` to write to file instead of nexus when daemon is not running. Implement direct Anthropic/Ollama client in hex-agent that does not require SpacetimeDB (the `anthropic.rs` adapter at 433 lines already does this — it is not SpacetimeDB-gated).

3. **Phase 3 (high risk, defer):** Swarm coordination and remote secrets distribution remain nexus-dependent. Do not attempt to replace these with hooks. Instead, scope the "minimal SpacetimeDB" to only: `agent_registry` + `hook_registry` + `inference_endpoint` tables — roughly the `fleet-state` WASM module (the smallest surface).

4. **Do not attempt:** Replacing `stdb_inference.rs` without first building a local non-SpacetimeDB inference adapter. The current code has an explicit hard failure path — a partial migration here would silently break inference in production.

---

## Summary Table

| Capability | Break Risk if Removed | Hooks Replacement Complexity | Recommendation |
|---|---|---|---|
| Arch enforcement (local) | Low — `enforce prompt` survives | Low (~50 lines) | Migrate to hooks |
| Arch analysis (treesitter) | Medium — loses CI integration | Low (invoke binary) | Migrate to hooks |
| Permissions (local env) | None — env vars always work | None needed | N/A |
| Permissions (remote grants) | **High** — remote agents blind | **Not feasible** | Keep nexus |
| Cost tracking | **High** — no fallback in stdb_inference | Medium (JSONL log) | Keep minimal StDB |
| Inference routing | **High** — hard failure by design | Medium | Keep inference module |
| Swarm coordination | **Critical** — 30+ tools broken | Not feasible | Keep nexus |
| Remote agent lifecycle | **High** — SSH/WS lifecycle gone | Not feasible | Keep nexus |

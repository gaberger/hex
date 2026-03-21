# Component: SpacetimeDB

## One-Line Summary

Coordination & state core — the required backbone service providing real-time WebSocket state synchronization across all hex clients via 18 WASM modules with transactional reducers.

## Key Facts

- **Required** — must always be running for hex to function (SQLite fallback for degraded single-node mode)
- Rust-native relational database with embedded WASM application logic
- All clients connect via WebSocket at `ws://localhost:3000`
- 18 WASM modules compiled from Rust to `wasm32-unknown-unknown`
- Reducers are transactional — atomic success or full rollback
- WASM sandbox: **no filesystem, no process spawning, no network calls**
- SDK version: `spacetimedb-sdk 2.0` (Rust), SpacetimeDB SDK v2.0 (TypeScript/client)

## WASM Modules

### Core Coordination

#### hexflo-coordination
The central module — manages swarms, tasks, agents, memory, projects, config, skills, and agent definitions.

**Tables:**
| Table | Columns | Purpose |
|:------|:--------|:--------|
| `swarm` | id, project_id, name, topology, status, created_at, updated_at | Swarm instances |
| `swarm_task` | id, swarm_id, title, status, agent_id, result, created_at, completed_at | Tasks within swarms |
| `swarm_agent` | id, swarm_id, name, role, status, worktree_path, last_heartbeat | Agents in swarms |
| `hexflo_memory` | key, value, scope, updated_at | Scoped key-value store (global/swarm/agent) |
| `project` | project_id, name, path, registered_at | Registered projects |
| `project_config` | key, project_id, value_json, source_file, synced_at | Project configuration |
| `skill_registry` | skill_id, project_id, name, trigger_cmd, description, source_path, synced_at | Synced skill definitions |
| `agent_definition` | agent_def_id, project_id, name, role, model, capabilities_json, tools_json, source_path, synced_at | Synced agent definitions |

**Reducers:**
| Reducer | Parameters | Purpose |
|:--------|:-----------|:--------|
| `register_project` | project_id, name, path | Register a project with SpacetimeDB |
| `remove_project` | project_id | Unregister a project |
| `sync_config` | key, project_id, value_json, source_file | Push config from repo to SpacetimeDB |
| `sync_skill` | skill_id, project_id, name, trigger_cmd, description, source_path | Sync a skill definition |
| `sync_agent_def` | agent_def_id, project_id, name, role, model, capabilities_json, tools_json, source_path | Sync an agent definition |
| `swarm_init` | id, project_id, name, topology | Create a new swarm |
| `swarm_complete` | id | Mark swarm as completed |
| `swarm_fail` | id | Mark swarm as failed |
| `task_create` | id, swarm_id, title | Create a task in a swarm |
| `task_assign` | id, agent_id | Assign task to agent |
| `task_complete` | id, result | Mark task completed with result |
| `task_fail` | id, result | Mark task as failed |
| `task_reclaim` | id | Reclaim task from dead agent |
| `agent_register` | id, swarm_id, name, role | Register agent in swarm |
| `agent_heartbeat` | id | Update agent heartbeat timestamp |
| `agent_mark_stale` | id | Mark agent as stale (45s no heartbeat) |
| `agent_mark_dead` | id | Mark agent as dead (120s no heartbeat) |
| `agent_remove` | id | Remove agent from swarm |
| `memory_store` | key, value, scope | Store key-value in scoped memory |
| `memory_delete` | key | Delete a memory entry |
| `memory_clear_scope` | scope | Clear all entries in a scope |

#### hexflo-cleanup (Scheduled)
Automated stale agent detection and task reclamation.

**Scheduled:** `run_cleanup` executes every 30 seconds.

**Tables:** agent_health, reclaimable_task, cleanup_log, cleanup_schedule

**Key Reducers:** run_cleanup (scheduled), trigger_cleanup (manual), remove_dead_agent

#### hexflo-lifecycle
Phase transition management for swarm feature lifecycle.

**Tables:** swarm_lifecycle, lifecycle_task, phase_transition_log

**Key Reducers:** on_task_complete (triggers automatic phase advance), check_unblocked_tasks

### Agent Management

#### agent-registry
Agent lifecycle tracking independent of swarm context.

**Tables:** agent (id, name, project_dir, model, status, metrics_json), agent_heartbeat

**Reducers:** register_agent, update_status, heartbeat, remove_agent

#### agent-definition-registry
Agent definition metadata with versioned audit trail.

**Tables:** agent_definition (name, role_prompt, allowed_tools, constraints, model), agent_definition_version (snapshot_json)

**Reducers:** register_definition, update_definition, remove_definition, get_definition_by_name

### Inference

#### inference-gateway
LLM request routing with provider health tracking and agent budgets.

**Tables:** inference_request, inference_response, inference_provider (health, rate limits), agent_budget (token/cost limits), inference_stream_chunk

**Key Reducers:** request_inference, complete_inference, fail_inference, register_provider, set_agent_budget

#### inference-bridge
Model integration with queue-based processing.

**Tables:** inference_queue (status, worker_id), inference_result, agent_token_budget, provider_route

**Key Reducers:** submit_inference, claim_inference, complete_inference, register_provider_route

### Architecture & Development

#### architecture-enforcer
Server-side boundary rule validation. Seeds 6 default hex rules on init.

**Tables:** boundary_rule (source_layer, forbidden_import, severity), write_validation (verdict, violations)

**Reducers:** seed_default_rules, validate_write

#### workplan-state
Task status and phase tracking for workplan execution.

**Tables:** workplan_execution (status, current_phase), workplan_task (layer, status, agent_id)

**Reducers:** start_workplan, update_task, advance_phase

#### skill-registry
Skill metadata with trigger indexing for fast lookup.

**Tables:** skill (name, description, triggers_json, body), skill_trigger_index

**Reducers:** register_skill, update_skill, remove_skill, search_skills

#### hook-registry
Hook management with execution logging.

**Tables:** hook (event_type, handler_type, blocking, tool_pattern), hook_execution_log

**Reducers:** register_hook, toggle_hook, log_execution

### Communication

#### chat-relay
Message routing between agents and humans.

**Tables:** conversation (agent_id, archived), message (role, sender_name, content)

**Reducers:** create_conversation, send_message, archive_conversation, clear_conversation

### Infrastructure

#### fleet-state
Compute node registry for remote agent deployment.

**Tables:** compute_node (host, port, status, max_agents, active_agents)

**Reducers:** register_node, update_health, increment_agents, decrement_agents

#### file-lock-manager
Distributed file locking for multi-agent coordination.

**Tables:** file_lock (file_path, agent_id, lock_type: exclusive|shared_read, worktree)

**Reducers:** acquire_lock, release_lock, expire_stale_locks

#### conflict-resolver
State conflict detection and resolution.

**Tables:** conflict_event (file_path, agents_json, resolution: pending|priority|merge|escalate)

**Reducers:** report_conflict, resolve_conflict

#### secret-grant (ADR-026)
Secret distribution with audit trail. Has both private and public tables.

**Tables:** secret_grant (private), inference_endpoint (public), secret_vault (public), secret_audit_log (private)

**Reducers:** grant_secret, claim_grant, revoke_secret, store_secret, audit_log

#### rl-engine
Reinforcement learning with Q-learning and pattern storage.

**Tables:** rl_experience, rl_q_entry (composite key: state_key::action), rl_pattern (confidence, decay_rate)

**Reducers:** select_action, record_reward, store_pattern, decay_patterns

## Configuration

**Location:** `.hex/state.json`
```json
{
  "backend": "spacetimedb",
  "spacetimedb": {
    "host": "localhost:3000",
    "database": "hex-nexus"
  }
}
```

**Client connection (TypeScript):**
```typescript
DbConnection.builder()
  .withUri("ws://localhost:3000")
  .withDatabaseName("hexflo-coordination")
  .onConnect((conn) => {
    conn.subscriptionBuilder()
      .subscribe(["SELECT * FROM swarm", "SELECT * FROM swarm_task"])
  })
  .build()
```

**Building modules:**
```bash
cd spacetime-modules
spacetime build           # Compile all WASM modules
spacetime publish hex-nexus  # Deploy to SpacetimeDB instance
```

## Depends On

- Nothing (SpacetimeDB is the foundation layer)

## Depended On By

- hex-nexus (state backend, reducer calls via HTTP)
- hex-dashboard (real-time subscriptions via WebSocket)
- hex-cli (state queries)
- hex-agent (coordination and enforcement state)
- hex-chat (message relay)
- hex-desktop (via dashboard)

## Related ADRs

- ADR-025: SpacetimeDB as Distributed State Backend
- ADR-026: Secret Management
- ADR-032b: SQLite-to-SpacetimeDB Migration
- ADR-044: Config Sync to SpacetimeDB

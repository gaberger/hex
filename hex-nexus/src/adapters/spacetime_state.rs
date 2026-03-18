//! SpacetimeDB-backed implementation of IStatePort.
//!
//! Two compilation modes:
//! 1. Default (no feature): Stub that returns connection errors.
//!    Used when SpacetimeDB is not available.
//! 2. `spacetimedb` feature: Real implementation using spacetimedb-sdk.
//!    Connects via WebSocket, calls reducers for writes, reads from
//!    subscription cache for queries.
//!
//! Enabled via `.hex/state.json`:
//! ```json
//! { "backend": "spacetimedb", "spacetimedb": { "host": "localhost:3000", "database": "hex-nexus" } }
//! ```

use async_trait::async_trait;
use tokio::sync::broadcast;

use crate::ports::state::*;

/// Configuration for connecting to SpacetimeDB.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SpacetimeConfig {
    pub host: String,
    pub database: String,
    pub auth_token: Option<String>,
}

impl Default for SpacetimeConfig {
    fn default() -> Self {
        Self {
            host: "http://localhost:3000".to_string(),
            database: "hex-nexus".to_string(),
            auth_token: None,
        }
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Feature-gated implementation (real SpacetimeDB SDK)
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[cfg(feature = "spacetimedb")]
mod real {
    use super::*;
    use std::sync::Arc;
    use tokio::sync::RwLock;

    /// SpacetimeDB-backed state adapter using the real SDK.
    ///
    /// Architecture:
    /// - Connects to SpacetimeDB via WebSocket (DbConnection::builder())
    /// - Writes: call reducers (e.g., conn.reducers().register_agent(...))
    /// - Reads: query the local subscription cache (e.g., conn.db().agent().iter())
    /// - Events: table callbacks (on_insert/on_delete/on_update) feed the broadcast channel
    pub struct SpacetimeStateAdapter {
        config: SpacetimeConfig,
        event_tx: broadcast::Sender<StateEvent>,
        connected: RwLock<bool>,
        // When generated bindings are available, this will hold:
        // connection: RwLock<Option<DbConnection>>,
    }

    impl SpacetimeStateAdapter {
        pub fn new(config: SpacetimeConfig) -> Self {
            let (event_tx, _) = broadcast::channel(256);
            Self {
                config,
                event_tx,
                connected: RwLock::new(false),
            }
        }

        /// Connect to SpacetimeDB and subscribe to all tables.
        ///
        /// Once generated bindings are available, this will:
        /// 1. DbConnection::builder()
        ///      .with_uri(&self.config.host)
        ///      .with_database_name(&self.config.database)
        ///      .on_connect(|ctx| {
        ///          ctx.subscription_builder()
        ///              .on_applied(|ctx| { /* cache ready */ })
        ///              .subscribe([
        ///                  "SELECT * FROM agent",
        ///                  "SELECT * FROM rl_q_entry",
        ///                  "SELECT * FROM rl_pattern",
        ///                  "SELECT * FROM workplan_task",
        ///                  "SELECT * FROM message",
        ///                  "SELECT * FROM compute_node",
        ///                  "SELECT * FROM skill",
        ///                  "SELECT * FROM hook",
        ///                  "SELECT * FROM agent_definition",
        ///              ]);
        ///      })
        ///      .build()
        /// 2. Register on_insert/on_update/on_delete callbacks to forward StateEvents
        /// 3. Store the connection handle for reducer calls
        pub async fn connect(&self) -> Result<(), StateError> {
            tracing::info!(
                host = %self.config.host,
                db = %self.config.database,
                "Connecting to SpacetimeDB"
            );

            // TODO: Replace with real DbConnection::builder() once generated bindings exist.
            // The generated code from `spacetime generate --lang rust` will provide:
            // - DbConnection type with .db() and .reducers()
            // - Table accessor types (agent, rl_q_entry, etc.)
            // - Reducer call methods (register_agent, select_action, etc.)
            //
            // For now, we verify the feature compiles and return a connection error
            // indicating that codegen hasn't been run yet.
            Err(StateError::Connection(
                "SpacetimeDB SDK linked but codegen bindings not yet generated. \
                 Run: spacetime generate --lang rust --out-dir hex-hub/src/spacetime_bindings/ \
                 --project-path spacetime-modules/<module>"
                    .into(),
            ))
        }

        fn not_connected() -> StateError {
            StateError::Connection("SpacetimeDB not connected".into())
        }
    }

    #[async_trait]
    impl IStatePort for SpacetimeStateAdapter {
        // ── RL ───────────────────────────────────────────
        // Maps to: rl-engine module reducers

        async fn rl_select_action(&self, _state: &RlState) -> Result<String, StateError> {
            // conn.reducers().select_action(state_key, epsilon)
            Err(Self::not_connected())
        }

        async fn rl_record_reward(
            &self,
            _state_key: &str,
            _action: &str,
            _reward: f64,
            _next_state_key: &str,
        ) -> Result<(), StateError> {
            // conn.reducers().record_reward(state_key, action, reward, next_state_key, outcome)
            Err(Self::not_connected())
        }

        async fn rl_get_stats(&self) -> Result<RlStats, StateError> {
            // Read from subscription cache: conn.db().rl_q_entry().iter().count() etc.
            Err(Self::not_connected())
        }

        // ── Patterns ────────────────────────────────────
        // Maps to: rl-engine module (rl_pattern table + store_pattern/decay_patterns reducers)

        async fn pattern_store(
            &self,
            _category: &str,
            _content: &str,
            _confidence: f64,
        ) -> Result<String, StateError> {
            // conn.reducers().store_pattern(id, category, content, confidence, timestamp)
            Err(Self::not_connected())
        }

        async fn pattern_search(
            &self,
            _category: &str,
            _query: &str,
            _limit: u32,
        ) -> Result<Vec<PatternEntry>, StateError> {
            // Read from cache: conn.db().rl_pattern().iter().filter(|p| p.category == category && p.content.contains(query))
            Err(Self::not_connected())
        }

        async fn pattern_reinforce(&self, _id: &str, _delta: f64) -> Result<(), StateError> {
            // conn.reducers().reinforce_pattern(id, delta) — needs a reducer added to rl-engine
            Err(Self::not_connected())
        }

        async fn pattern_decay_all(&self) -> Result<u32, StateError> {
            // conn.reducers().decay_patterns(factor, timestamp)
            Err(Self::not_connected())
        }

        // ── Agent Registry ──────────────────────────────
        // Maps to: agent-registry module

        async fn agent_register(&self, _info: AgentInfo) -> Result<String, StateError> {
            // conn.reducers().register_agent(id, name, project_dir, model, timestamp)
            Err(Self::not_connected())
        }

        async fn agent_update_status(
            &self,
            _id: &str,
            _status: AgentStatus,
            _metrics: Option<AgentMetricsData>,
        ) -> Result<(), StateError> {
            // conn.reducers().update_status(id, status_str, input_tokens, output_tokens, tool_calls, turns, timestamp)
            Err(Self::not_connected())
        }

        async fn agent_list(&self) -> Result<Vec<AgentInfo>, StateError> {
            // conn.db().agent().iter().map(|a| AgentInfo { ... }).collect()
            Err(Self::not_connected())
        }

        async fn agent_get(&self, _id: &str) -> Result<Option<AgentInfo>, StateError> {
            // conn.db().agent().id().find(id).map(|a| AgentInfo { ... })
            Err(Self::not_connected())
        }

        async fn agent_remove(&self, _id: &str) -> Result<(), StateError> {
            // conn.reducers().remove_agent(id)
            Err(Self::not_connected())
        }

        // ── Workplan ────────────────────────────────────
        // Maps to: workplan-state module

        async fn workplan_update_task(&self, _update: WorkplanTaskUpdate) -> Result<(), StateError> {
            // conn.reducers().update_task(execution_id, task_id, status, agent_id, result, timestamp)
            Err(Self::not_connected())
        }

        async fn workplan_get_tasks(
            &self,
            _workplan_id: &str,
        ) -> Result<Vec<WorkplanTaskUpdate>, StateError> {
            // conn.db().workplan_task().iter().filter(|t| t.execution_id == workplan_id)
            Err(Self::not_connected())
        }

        // ── Chat ────────────────────────────────────────
        // Maps to: chat-relay module

        async fn chat_send(&self, _message: ChatMessage) -> Result<(), StateError> {
            // conn.reducers().send_message(conversation_id, role, sender_name, content)
            Err(Self::not_connected())
        }

        async fn chat_history(
            &self,
            _conversation_id: &str,
            _limit: u32,
        ) -> Result<Vec<ChatMessage>, StateError> {
            // conn.db().message().iter()
            //     .filter(|m| m.conversation_id == conversation_id)
            //     .map(|m| ChatMessage {
            //         id: m.id, conversation_id: m.conversation_id,
            //         role: m.role, sender_name: m.sender_name,
            //         content: m.content, timestamp: m.timestamp,
            //     })
            //     .take(limit)
            Err(Self::not_connected())
        }

        // ── Fleet ───────────────────────────────────────
        // Maps to: fleet-state module

        async fn fleet_register(&self, _node: FleetNode) -> Result<(), StateError> {
            // conn.reducers().register_node(id, host, port, max_agents, timestamp)
            Err(Self::not_connected())
        }

        async fn fleet_update_status(&self, _id: &str, _status: &str) -> Result<(), StateError> {
            // conn.reducers().update_health(id, status, timestamp)
            Err(Self::not_connected())
        }

        async fn fleet_list(&self) -> Result<Vec<FleetNode>, StateError> {
            // conn.db().compute_node().iter().map(...)
            Err(Self::not_connected())
        }

        async fn fleet_remove(&self, _id: &str) -> Result<(), StateError> {
            // conn.reducers().remove_node(id)
            Err(Self::not_connected())
        }

        // ── Skill Registry ────────────────────────────────
        // Maps to: skill-registry module

        async fn skill_register(&self, _skill: SkillEntry) -> Result<String, StateError> {
            // conn.reducers().register_skill(id, name, description, triggers_json, body, source, timestamp)
            Err(Self::not_connected())
        }

        async fn skill_update(&self, _id: &str, _description: &str, _triggers_json: &str, _body: &str) -> Result<(), StateError> {
            // conn.reducers().update_skill(id, description, triggers_json, body, timestamp)
            Err(Self::not_connected())
        }

        async fn skill_remove(&self, _id: &str) -> Result<(), StateError> {
            // conn.reducers().remove_skill(id)
            Err(Self::not_connected())
        }

        async fn skill_list(&self) -> Result<Vec<SkillEntry>, StateError> {
            // conn.db().skill().iter().map(...)
            Err(Self::not_connected())
        }

        async fn skill_get(&self, _id: &str) -> Result<Option<SkillEntry>, StateError> {
            // conn.db().skill().id().find(id).map(...)
            Err(Self::not_connected())
        }

        async fn skill_search(&self, _trigger_type: &str, _query: &str) -> Result<Vec<SkillEntry>, StateError> {
            // conn.db().skill_trigger_index().iter().filter(...)
            Err(Self::not_connected())
        }

        // ── Hook Registry ──────────────────────────────────
        // Maps to: hook-registry module

        async fn hook_register(&self, _hook: HookEntry) -> Result<String, StateError> {
            // conn.reducers().register_hook(...)
            Err(Self::not_connected())
        }

        async fn hook_update(&self, _id: &str, _handler_config_json: &str, _timeout_secs: u32, _blocking: bool, _tool_pattern: &str) -> Result<(), StateError> {
            // conn.reducers().update_hook(...)
            Err(Self::not_connected())
        }

        async fn hook_remove(&self, _id: &str) -> Result<(), StateError> {
            Err(Self::not_connected())
        }

        async fn hook_toggle(&self, _id: &str, _enabled: bool) -> Result<(), StateError> {
            // conn.reducers().toggle_hook(id, enabled, timestamp)
            Err(Self::not_connected())
        }

        async fn hook_list(&self) -> Result<Vec<HookEntry>, StateError> {
            Err(Self::not_connected())
        }

        async fn hook_list_by_event(&self, _event_type: &str) -> Result<Vec<HookEntry>, StateError> {
            // conn.db().hook().iter().filter(|h| h.event_type == event_type && h.enabled)
            Err(Self::not_connected())
        }

        async fn hook_log_execution(&self, _entry: HookExecutionEntry) -> Result<(), StateError> {
            // conn.reducers().log_execution(...)
            Err(Self::not_connected())
        }

        // ── Agent Definition Registry ──────────────────────
        // Maps to: agent-definition-registry module

        async fn agent_def_register(&self, _def: AgentDefinitionEntry) -> Result<String, StateError> {
            // conn.reducers().register_definition(...)
            Err(Self::not_connected())
        }

        async fn agent_def_update(
            &self, _id: &str, _description: &str, _role_prompt: &str,
            _allowed_tools_json: &str, _constraints_json: &str, _model: &str,
            _max_turns: u32, _metadata_json: &str,
        ) -> Result<(), StateError> {
            // conn.reducers().update_definition(...)
            Err(Self::not_connected())
        }

        async fn agent_def_remove(&self, _id: &str) -> Result<(), StateError> {
            // conn.reducers().remove_definition(id)
            Err(Self::not_connected())
        }

        async fn agent_def_list(&self) -> Result<Vec<AgentDefinitionEntry>, StateError> {
            // conn.db().agent_definition().iter().map(...)
            Err(Self::not_connected())
        }

        async fn agent_def_get_by_name(&self, _name: &str) -> Result<Option<AgentDefinitionEntry>, StateError> {
            // conn.db().agent_definition().name().find(name).map(...)
            Err(Self::not_connected())
        }

        async fn agent_def_versions(&self, _definition_id: &str) -> Result<Vec<AgentDefinitionVersionEntry>, StateError> {
            // conn.db().agent_definition_version().iter().filter(|v| v.definition_id == definition_id)
            Err(Self::not_connected())
        }

        // ── HexFlo Coordination ──────────────────────────
        // Maps to: hexflo-coordination module

        async fn swarm_init(&self, _id: &str, _name: &str, _topology: &str, _project_id: &str) -> Result<(), StateError> {
            // POST /api/swarms { id, name, topology, project_id }
            Err(Self::not_connected())
        }

        async fn swarm_complete(&self, _id: &str) -> Result<(), StateError> {
            // PATCH /api/swarms/:id { status: "completed" }
            Err(Self::not_connected())
        }

        async fn swarm_fail(&self, _id: &str, _reason: &str) -> Result<(), StateError> {
            // PATCH /api/swarms/:id { status: "failed", reason }
            Err(Self::not_connected())
        }

        async fn swarm_list_active(&self) -> Result<Vec<SwarmInfo>, StateError> {
            // GET /api/swarms
            Err(Self::not_connected())
        }

        async fn swarm_task_create(&self, _id: &str, _swarm_id: &str, _title: &str) -> Result<(), StateError> {
            // POST /api/swarms/:swarm_id/tasks { id, title }
            Err(Self::not_connected())
        }

        async fn swarm_task_assign(&self, _task_id: &str, _agent_id: &str) -> Result<(), StateError> {
            // PATCH /api/swarms/tasks/:task_id { agent_id }
            Err(Self::not_connected())
        }

        async fn swarm_task_complete(&self, _task_id: &str, _result: &str) -> Result<(), StateError> {
            // PATCH /api/swarms/tasks/:task_id { status: "completed", result }
            Err(Self::not_connected())
        }

        async fn swarm_task_fail(&self, _task_id: &str, _reason: &str) -> Result<(), StateError> {
            // PATCH /api/swarms/tasks/:task_id { status: "failed", reason }
            Err(Self::not_connected())
        }

        async fn swarm_task_list(&self, _swarm_id: Option<&str>) -> Result<Vec<SwarmTaskInfo>, StateError> {
            // GET /api/swarms or GET /api/swarms/:swarm_id/tasks
            Err(Self::not_connected())
        }

        async fn swarm_agent_register(&self, _id: &str, _swarm_id: &str, _name: &str, _role: &str, _worktree_path: &str) -> Result<(), StateError> {
            // POST /api/swarms/:swarm_id/agents { id, name, role, worktree_path }
            Err(Self::not_connected())
        }

        async fn swarm_agent_heartbeat(&self, _id: &str) -> Result<(), StateError> {
            // POST /api/swarms/agents/:id/heartbeat
            Err(Self::not_connected())
        }

        async fn swarm_agent_remove(&self, _id: &str) -> Result<(), StateError> {
            // DELETE /api/swarms/agents/:id
            Err(Self::not_connected())
        }

        async fn swarm_cleanup_stale(&self, _stale_secs: u64, _dead_secs: u64) -> Result<CleanupReport, StateError> {
            // POST /api/hexflo/cleanup { stale_secs, dead_secs }
            Err(Self::not_connected())
        }

        async fn hexflo_memory_store(&self, _key: &str, _value: &str, _scope: &str) -> Result<(), StateError> {
            // POST /api/hexflo/memory { key, value, scope }
            Err(Self::not_connected())
        }

        async fn hexflo_memory_retrieve(&self, _key: &str) -> Result<Option<String>, StateError> {
            // GET /api/hexflo/memory/:key
            Err(Self::not_connected())
        }

        async fn hexflo_memory_search(&self, _query: &str) -> Result<Vec<(String, String)>, StateError> {
            // GET /api/hexflo/memory/search?q=query
            Err(Self::not_connected())
        }

        async fn hexflo_memory_delete(&self, _key: &str) -> Result<(), StateError> {
            // DELETE /api/hexflo/memory/:key
            Err(Self::not_connected())
        }

        // ── Subscriptions ───────────────────────────────
        // SpacetimeDB forwards table change callbacks through this channel

        fn subscribe(&self) -> broadcast::Receiver<StateEvent> {
            self.event_tx.subscribe()
        }
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Stub implementation (no SpacetimeDB SDK)
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[cfg(not(feature = "spacetimedb"))]
mod stub {
    use super::*;

    pub struct SpacetimeStateAdapter {
        config: SpacetimeConfig,
        event_tx: broadcast::Sender<StateEvent>,
    }

    impl SpacetimeStateAdapter {
        pub fn new(config: SpacetimeConfig) -> Self {
            let (event_tx, _) = broadcast::channel(256);
            Self { config, event_tx }
        }

        pub async fn connect(&self) -> Result<(), StateError> {
            tracing::info!(host = %self.config.host, db = %self.config.database, "SpacetimeDB feature not enabled");
            Err(StateError::Connection("SpacetimeDB not compiled — rebuild with --features spacetimedb".into()))
        }

        fn err() -> StateError { StateError::Connection("SpacetimeDB not compiled".into()) }
    }

    #[async_trait]
    impl IStatePort for SpacetimeStateAdapter {
        async fn rl_select_action(&self, _: &RlState) -> Result<String, StateError> { Err(Self::err()) }
        async fn rl_record_reward(&self, _: &str, _: &str, _: f64, _: &str) -> Result<(), StateError> { Err(Self::err()) }
        async fn rl_get_stats(&self) -> Result<RlStats, StateError> { Err(Self::err()) }
        async fn pattern_store(&self, _: &str, _: &str, _: f64) -> Result<String, StateError> { Err(Self::err()) }
        async fn pattern_search(&self, _: &str, _: &str, _: u32) -> Result<Vec<PatternEntry>, StateError> { Err(Self::err()) }
        async fn pattern_reinforce(&self, _: &str, _: f64) -> Result<(), StateError> { Err(Self::err()) }
        async fn pattern_decay_all(&self) -> Result<u32, StateError> { Err(Self::err()) }
        async fn agent_register(&self, _: AgentInfo) -> Result<String, StateError> { Err(Self::err()) }
        async fn agent_update_status(&self, _: &str, _: AgentStatus, _: Option<AgentMetricsData>) -> Result<(), StateError> { Err(Self::err()) }
        async fn agent_list(&self) -> Result<Vec<AgentInfo>, StateError> { Err(Self::err()) }
        async fn agent_get(&self, _: &str) -> Result<Option<AgentInfo>, StateError> { Err(Self::err()) }
        async fn agent_remove(&self, _: &str) -> Result<(), StateError> { Err(Self::err()) }
        async fn workplan_update_task(&self, _: WorkplanTaskUpdate) -> Result<(), StateError> { Err(Self::err()) }
        async fn workplan_get_tasks(&self, _: &str) -> Result<Vec<WorkplanTaskUpdate>, StateError> { Err(Self::err()) }
        async fn chat_send(&self, _: ChatMessage) -> Result<(), StateError> { Err(Self::err()) }
        async fn chat_history(&self, _: &str, _: u32) -> Result<Vec<ChatMessage>, StateError> { Err(Self::err()) }
        async fn fleet_register(&self, _: FleetNode) -> Result<(), StateError> { Err(Self::err()) }
        async fn fleet_update_status(&self, _: &str, _: &str) -> Result<(), StateError> { Err(Self::err()) }
        async fn fleet_list(&self) -> Result<Vec<FleetNode>, StateError> { Err(Self::err()) }
        async fn fleet_remove(&self, _: &str) -> Result<(), StateError> { Err(Self::err()) }
        async fn skill_register(&self, _: SkillEntry) -> Result<String, StateError> { Err(Self::err()) }
        async fn skill_update(&self, _: &str, _: &str, _: &str, _: &str) -> Result<(), StateError> { Err(Self::err()) }
        async fn skill_remove(&self, _: &str) -> Result<(), StateError> { Err(Self::err()) }
        async fn skill_list(&self) -> Result<Vec<SkillEntry>, StateError> { Err(Self::err()) }
        async fn skill_get(&self, _: &str) -> Result<Option<SkillEntry>, StateError> { Err(Self::err()) }
        async fn skill_search(&self, _: &str, _: &str) -> Result<Vec<SkillEntry>, StateError> { Err(Self::err()) }
        async fn hook_register(&self, _: HookEntry) -> Result<String, StateError> { Err(Self::err()) }
        async fn hook_update(&self, _: &str, _: &str, _: u32, _: bool, _: &str) -> Result<(), StateError> { Err(Self::err()) }
        async fn hook_remove(&self, _: &str) -> Result<(), StateError> { Err(Self::err()) }
        async fn hook_toggle(&self, _: &str, _: bool) -> Result<(), StateError> { Err(Self::err()) }
        async fn hook_list(&self) -> Result<Vec<HookEntry>, StateError> { Err(Self::err()) }
        async fn hook_list_by_event(&self, _: &str) -> Result<Vec<HookEntry>, StateError> { Err(Self::err()) }
        async fn hook_log_execution(&self, _: HookExecutionEntry) -> Result<(), StateError> { Err(Self::err()) }
        async fn agent_def_register(&self, _: AgentDefinitionEntry) -> Result<String, StateError> { Err(Self::err()) }
        async fn agent_def_update(&self, _: &str, _: &str, _: &str, _: &str, _: &str, _: &str, _: u32, _: &str) -> Result<(), StateError> { Err(Self::err()) }
        async fn agent_def_remove(&self, _: &str) -> Result<(), StateError> { Err(Self::err()) }
        async fn agent_def_list(&self) -> Result<Vec<AgentDefinitionEntry>, StateError> { Err(Self::err()) }
        async fn agent_def_get_by_name(&self, _: &str) -> Result<Option<AgentDefinitionEntry>, StateError> { Err(Self::err()) }
        async fn agent_def_versions(&self, _: &str) -> Result<Vec<AgentDefinitionVersionEntry>, StateError> { Err(Self::err()) }
        async fn swarm_init(&self, _: &str, _: &str, _: &str, _: &str) -> Result<(), StateError> { Err(Self::err()) }
        async fn swarm_complete(&self, _: &str) -> Result<(), StateError> { Err(Self::err()) }
        async fn swarm_fail(&self, _: &str, _: &str) -> Result<(), StateError> { Err(Self::err()) }
        async fn swarm_list_active(&self) -> Result<Vec<SwarmInfo>, StateError> { Err(Self::err()) }
        async fn swarm_task_create(&self, _: &str, _: &str, _: &str) -> Result<(), StateError> { Err(Self::err()) }
        async fn swarm_task_assign(&self, _: &str, _: &str) -> Result<(), StateError> { Err(Self::err()) }
        async fn swarm_task_complete(&self, _: &str, _: &str) -> Result<(), StateError> { Err(Self::err()) }
        async fn swarm_task_fail(&self, _: &str, _: &str) -> Result<(), StateError> { Err(Self::err()) }
        async fn swarm_task_list(&self, _: Option<&str>) -> Result<Vec<SwarmTaskInfo>, StateError> { Err(Self::err()) }
        async fn swarm_agent_register(&self, _: &str, _: &str, _: &str, _: &str, _: &str) -> Result<(), StateError> { Err(Self::err()) }
        async fn swarm_agent_heartbeat(&self, _: &str) -> Result<(), StateError> { Err(Self::err()) }
        async fn swarm_agent_remove(&self, _: &str) -> Result<(), StateError> { Err(Self::err()) }
        async fn swarm_cleanup_stale(&self, _: u64, _: u64) -> Result<CleanupReport, StateError> { Err(Self::err()) }
        async fn hexflo_memory_store(&self, _: &str, _: &str, _: &str) -> Result<(), StateError> { Err(Self::err()) }
        async fn hexflo_memory_retrieve(&self, _: &str) -> Result<Option<String>, StateError> { Err(Self::err()) }
        async fn hexflo_memory_search(&self, _: &str) -> Result<Vec<(String, String)>, StateError> { Err(Self::err()) }
        async fn hexflo_memory_delete(&self, _: &str) -> Result<(), StateError> { Err(Self::err()) }
        fn subscribe(&self) -> broadcast::Receiver<StateEvent> { self.event_tx.subscribe() }
    }
}

#[cfg(feature = "spacetimedb")]
pub use real::SpacetimeStateAdapter;
#[cfg(not(feature = "spacetimedb"))]
pub use stub::SpacetimeStateAdapter;

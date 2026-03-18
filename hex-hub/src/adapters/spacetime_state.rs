//! SpacetimeDB-backed implementation of IStatePort.
//!
//! Connects to a SpacetimeDB instance via the Rust SDK, calls reducers,
//! and subscribes to table changes for real-time sync.
//!
//! This adapter is opt-in — enabled via `.hex/state.json` config:
//! ```json
//! { "backend": "spacetimedb", "spacetimedb": { "host": "localhost:3000", "database": "hex-nexus" } }
//! ```
//!
//! NOTE: This is a stub implementation. Full implementation requires the
//! `spacetimedb-sdk` crate and a running SpacetimeDB instance.
//! Compile with `--features spacetimedb` to enable.

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

/// SpacetimeDB-backed state adapter.
///
/// When fully implemented, this will:
/// 1. Connect to SpacetimeDB via WebSocket
/// 2. Call reducers for state mutations
/// 3. Subscribe to tables for real-time change events
/// 4. Forward change events through the broadcast channel
pub struct SpacetimeStateAdapter {
    config: SpacetimeConfig,
    event_tx: broadcast::Sender<StateEvent>,
}

impl SpacetimeStateAdapter {
    pub fn new(config: SpacetimeConfig) -> Self {
        let (event_tx, _) = broadcast::channel(256);
        Self { config, event_tx }
    }

    /// Connect to SpacetimeDB and subscribe to all tables.
    pub async fn connect(&self) -> Result<(), StateError> {
        tracing::info!(
            host = %self.config.host,
            db = %self.config.database,
            "Connecting to SpacetimeDB"
        );
        // TODO: spacetimedb_sdk::DbConnection::builder()
        //   .with_uri(&self.config.host)
        //   .with_module_name(&self.config.database)
        //   .on_connect(|_, _| { tracing::info!("Connected to SpacetimeDB"); })
        //   .build()
        Err(StateError::Connection(
            "SpacetimeDB adapter not yet implemented — install spacetimedb-sdk and enable feature flag".into(),
        ))
    }
}

#[async_trait]
impl IStatePort for SpacetimeStateAdapter {
    // ── RL ───────────────────────────────────────────

    async fn rl_select_action(&self, _state: &RlState) -> Result<String, StateError> {
        // TODO: call rl_engine::select_action reducer
        Err(StateError::Connection("SpacetimeDB not connected".into()))
    }

    async fn rl_record_reward(
        &self,
        _state_key: &str,
        _action: &str,
        _reward: f64,
        _next_state_key: &str,
    ) -> Result<(), StateError> {
        Err(StateError::Connection("SpacetimeDB not connected".into()))
    }

    async fn rl_get_stats(&self) -> Result<RlStats, StateError> {
        Err(StateError::Connection("SpacetimeDB not connected".into()))
    }

    // ── Patterns ────────────────────────────────────

    async fn pattern_store(
        &self,
        _category: &str,
        _content: &str,
        _confidence: f64,
    ) -> Result<String, StateError> {
        Err(StateError::Connection("SpacetimeDB not connected".into()))
    }

    async fn pattern_search(
        &self,
        _category: &str,
        _query: &str,
        _limit: u32,
    ) -> Result<Vec<PatternEntry>, StateError> {
        Err(StateError::Connection("SpacetimeDB not connected".into()))
    }

    async fn pattern_reinforce(&self, _id: &str, _delta: f64) -> Result<(), StateError> {
        Err(StateError::Connection("SpacetimeDB not connected".into()))
    }

    async fn pattern_decay_all(&self) -> Result<u32, StateError> {
        Err(StateError::Connection("SpacetimeDB not connected".into()))
    }

    // ── Agent Registry ──────────────────────────────

    async fn agent_register(&self, _info: AgentInfo) -> Result<String, StateError> {
        Err(StateError::Connection("SpacetimeDB not connected".into()))
    }

    async fn agent_update_status(
        &self,
        _id: &str,
        _status: AgentStatus,
        _metrics: Option<AgentMetricsData>,
    ) -> Result<(), StateError> {
        Err(StateError::Connection("SpacetimeDB not connected".into()))
    }

    async fn agent_list(&self) -> Result<Vec<AgentInfo>, StateError> {
        Err(StateError::Connection("SpacetimeDB not connected".into()))
    }

    async fn agent_get(&self, _id: &str) -> Result<Option<AgentInfo>, StateError> {
        Err(StateError::Connection("SpacetimeDB not connected".into()))
    }

    async fn agent_remove(&self, _id: &str) -> Result<(), StateError> {
        Err(StateError::Connection("SpacetimeDB not connected".into()))
    }

    // ── Workplan ────────────────────────────────────

    async fn workplan_update_task(&self, _update: WorkplanTaskUpdate) -> Result<(), StateError> {
        Err(StateError::Connection("SpacetimeDB not connected".into()))
    }

    async fn workplan_get_tasks(
        &self,
        _workplan_id: &str,
    ) -> Result<Vec<WorkplanTaskUpdate>, StateError> {
        Err(StateError::Connection("SpacetimeDB not connected".into()))
    }

    // ── Chat ────────────────────────────────────────

    async fn chat_send(&self, _message: ChatMessage) -> Result<(), StateError> {
        Err(StateError::Connection("SpacetimeDB not connected".into()))
    }

    async fn chat_history(
        &self,
        _conversation_id: &str,
        _limit: u32,
    ) -> Result<Vec<ChatMessage>, StateError> {
        Err(StateError::Connection("SpacetimeDB not connected".into()))
    }

    // ── Fleet ───────────────────────────────────────

    async fn fleet_register(&self, _node: FleetNode) -> Result<(), StateError> {
        Err(StateError::Connection("SpacetimeDB not connected".into()))
    }

    async fn fleet_update_status(&self, _id: &str, _status: &str) -> Result<(), StateError> {
        Err(StateError::Connection("SpacetimeDB not connected".into()))
    }

    async fn fleet_list(&self) -> Result<Vec<FleetNode>, StateError> {
        Err(StateError::Connection("SpacetimeDB not connected".into()))
    }

    async fn fleet_remove(&self, _id: &str) -> Result<(), StateError> {
        Err(StateError::Connection("SpacetimeDB not connected".into()))
    }

    // ── Subscriptions ───────────────────────────────

    fn subscribe(&self) -> broadcast::Receiver<StateEvent> {
        self.event_tx.subscribe()
    }
}

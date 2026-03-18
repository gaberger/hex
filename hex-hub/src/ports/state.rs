use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

// ── Domain Types ────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RlState {
    pub task_type: String,
    pub codebase_size: u64,
    pub agent_count: u8,
    pub token_usage: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RlStats {
    pub q_table_size: usize,
    pub avg_q_value: f64,
    pub epsilon: f64,
    pub total_experiences: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PatternEntry {
    pub id: String,
    pub category: String,
    pub content: String,
    pub confidence: f64,
    pub access_count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentInfo {
    pub id: String,
    pub name: String,
    pub project_dir: String,
    pub model: String,
    pub status: AgentStatus,
    pub started_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentStatus {
    Spawning,
    Running,
    Completed,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentMetricsData {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub tool_calls: u32,
    pub turns: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkplanTaskUpdate {
    pub task_id: String,
    pub status: String,
    pub agent_id: Option<String>,
    pub result: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub id: String,
    pub conversation_id: String,
    pub role: String,
    pub content: String,
    pub timestamp: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FleetNode {
    pub id: String,
    pub host: String,
    pub port: u16,
    pub status: String,
    pub active_agents: u32,
    pub max_agents: u32,
    pub last_health_check: Option<String>,
}

// ── State Change Events (for subscriptions) ─────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum StateEvent {
    #[serde(rename = "agent_changed")]
    AgentChanged { agent: AgentInfo },
    #[serde(rename = "task_changed")]
    TaskChanged { update: WorkplanTaskUpdate },
    #[serde(rename = "chat_message")]
    ChatMessage { message: ChatMessage },
    #[serde(rename = "fleet_changed")]
    FleetChanged { node: FleetNode },
}

// ── The Port ────────────────────────────────────────────

/// Unified state port — abstracts the storage + sync backend.
///
/// Two implementations:
/// 1. SqliteStateAdapter (default) — wraps existing RL, orchestration, fleet SQLite code
/// 2. SpacetimeStateAdapter (opt-in) — connects to SpacetimeDB with real-time subscriptions
#[async_trait]
pub trait IStatePort: Send + Sync {
    // ── RL Engine ───────────────────────────────────
    async fn rl_select_action(&self, state: &RlState) -> Result<String, StateError>;
    async fn rl_record_reward(
        &self,
        state_key: &str,
        action: &str,
        reward: f64,
        next_state_key: &str,
    ) -> Result<(), StateError>;
    async fn rl_get_stats(&self) -> Result<RlStats, StateError>;

    // ── Patterns ────────────────────────────────────
    async fn pattern_store(
        &self,
        category: &str,
        content: &str,
        confidence: f64,
    ) -> Result<String, StateError>;
    async fn pattern_search(
        &self,
        category: &str,
        query: &str,
        limit: u32,
    ) -> Result<Vec<PatternEntry>, StateError>;
    async fn pattern_reinforce(&self, id: &str, delta: f64) -> Result<(), StateError>;
    async fn pattern_decay_all(&self) -> Result<u32, StateError>;

    // ── Agent Registry ──────────────────────────────
    async fn agent_register(&self, info: AgentInfo) -> Result<String, StateError>;
    async fn agent_update_status(
        &self,
        id: &str,
        status: AgentStatus,
        metrics: Option<AgentMetricsData>,
    ) -> Result<(), StateError>;
    async fn agent_list(&self) -> Result<Vec<AgentInfo>, StateError>;
    async fn agent_get(&self, id: &str) -> Result<Option<AgentInfo>, StateError>;
    async fn agent_remove(&self, id: &str) -> Result<(), StateError>;

    // ── Workplan ────────────────────────────────────
    async fn workplan_update_task(&self, update: WorkplanTaskUpdate) -> Result<(), StateError>;
    async fn workplan_get_tasks(
        &self,
        workplan_id: &str,
    ) -> Result<Vec<WorkplanTaskUpdate>, StateError>;

    // ── Chat ────────────────────────────────────────
    async fn chat_send(&self, message: ChatMessage) -> Result<(), StateError>;
    async fn chat_history(
        &self,
        conversation_id: &str,
        limit: u32,
    ) -> Result<Vec<ChatMessage>, StateError>;

    // ── Fleet ───────────────────────────────────────
    async fn fleet_register(&self, node: FleetNode) -> Result<(), StateError>;
    async fn fleet_update_status(&self, id: &str, status: &str) -> Result<(), StateError>;
    async fn fleet_list(&self) -> Result<Vec<FleetNode>, StateError>;
    async fn fleet_remove(&self, id: &str) -> Result<(), StateError>;

    // ── Subscriptions (real-time sync) ──────────────
    fn subscribe(&self) -> broadcast::Receiver<StateEvent>;
}

#[derive(Debug, thiserror::Error)]
pub enum StateError {
    #[error("Storage error: {0}")]
    Storage(String),
    #[error("Not found: {0}")]
    NotFound(String),
    #[error("Connection error: {0}")]
    Connection(String),
    #[error("Serialization error: {0}")]
    Serialization(String),
}

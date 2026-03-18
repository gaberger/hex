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
    pub sender_name: String,
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

// ── Skill Registry Types ────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillEntry {
    pub id: String,
    pub name: String,
    pub description: String,
    /// JSON-encoded trigger definitions
    pub triggers_json: String,
    pub body: String,
    pub source: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillTriggerEntry {
    pub trigger_type: String,
    pub trigger_value: String,
}

// ── Hook Registry Types ─────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HookEntry {
    pub id: String,
    pub event_type: String,
    pub handler_type: String,
    pub handler_config_json: String,
    pub timeout_secs: u32,
    pub blocking: bool,
    pub tool_pattern: String,
    pub enabled: bool,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HookExecutionEntry {
    pub hook_id: String,
    pub agent_id: String,
    pub event_type: String,
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
    pub duration_ms: u64,
    pub timed_out: bool,
    pub timestamp: String,
}

// ── Agent Definition Registry Types ─────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentDefinitionEntry {
    pub id: String,
    pub name: String,
    pub description: String,
    pub role_prompt: String,
    pub allowed_tools_json: String,
    pub constraints_json: String,
    pub model: String,
    pub max_turns: u32,
    pub metadata_json: String,
    pub version: u32,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentDefinitionVersionEntry {
    pub definition_id: String,
    pub version: u32,
    pub snapshot_json: String,
    pub created_at: String,
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
    #[serde(rename = "skill_changed")]
    SkillChanged { skill: SkillEntry },
    #[serde(rename = "hook_changed")]
    HookChanged { hook: HookEntry },
    #[serde(rename = "agent_definition_changed")]
    AgentDefinitionChanged { definition: AgentDefinitionEntry },
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

    // ── Skill Registry ────────────────────────────────
    async fn skill_register(&self, skill: SkillEntry) -> Result<String, StateError>;
    async fn skill_update(
        &self,
        id: &str,
        description: &str,
        triggers_json: &str,
        body: &str,
    ) -> Result<(), StateError>;
    async fn skill_remove(&self, id: &str) -> Result<(), StateError>;
    async fn skill_list(&self) -> Result<Vec<SkillEntry>, StateError>;
    async fn skill_get(&self, id: &str) -> Result<Option<SkillEntry>, StateError>;
    async fn skill_search(
        &self,
        trigger_type: &str,
        query: &str,
    ) -> Result<Vec<SkillEntry>, StateError>;

    // ── Hook Registry ──────────────────────────────────
    async fn hook_register(&self, hook: HookEntry) -> Result<String, StateError>;
    async fn hook_update(
        &self,
        id: &str,
        handler_config_json: &str,
        timeout_secs: u32,
        blocking: bool,
        tool_pattern: &str,
    ) -> Result<(), StateError>;
    async fn hook_remove(&self, id: &str) -> Result<(), StateError>;
    async fn hook_toggle(&self, id: &str, enabled: bool) -> Result<(), StateError>;
    async fn hook_list(&self) -> Result<Vec<HookEntry>, StateError>;
    async fn hook_list_by_event(&self, event_type: &str) -> Result<Vec<HookEntry>, StateError>;
    async fn hook_log_execution(&self, entry: HookExecutionEntry) -> Result<(), StateError>;

    // ── Agent Definition Registry ──────────────────────
    async fn agent_def_register(&self, def: AgentDefinitionEntry) -> Result<String, StateError>;
    async fn agent_def_update(
        &self,
        id: &str,
        description: &str,
        role_prompt: &str,
        allowed_tools_json: &str,
        constraints_json: &str,
        model: &str,
        max_turns: u32,
        metadata_json: &str,
    ) -> Result<(), StateError>;
    async fn agent_def_remove(&self, id: &str) -> Result<(), StateError>;
    async fn agent_def_list(&self) -> Result<Vec<AgentDefinitionEntry>, StateError>;
    async fn agent_def_get_by_name(&self, name: &str) -> Result<Option<AgentDefinitionEntry>, StateError>;
    async fn agent_def_versions(
        &self,
        definition_id: &str,
    ) -> Result<Vec<AgentDefinitionVersionEntry>, StateError>;

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

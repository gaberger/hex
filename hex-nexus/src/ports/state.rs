use async_trait::async_trait;
use serde::{Deserialize, Deserializer, Serialize};
use tokio::sync::broadcast;

/// Deserialize a timestamp that may be an integer (millis) or an RFC3339 string.
fn deserialize_flexible_timestamp<'de, D>(deserializer: D) -> Result<i64, D::Error>
where
    D: Deserializer<'de>,
{
    use serde::de;
    struct TimestampVisitor;
    impl<'de> de::Visitor<'de> for TimestampVisitor {
        type Value = i64;
        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            f.write_str("an integer or RFC3339 timestamp string")
        }
        fn visit_i64<E: de::Error>(self, v: i64) -> Result<i64, E> { Ok(v) }
        fn visit_u64<E: de::Error>(self, v: u64) -> Result<i64, E> { Ok(v as i64) }
        fn visit_str<E: de::Error>(self, v: &str) -> Result<i64, E> {
            // Try numeric string first ("0", "1234567890000"), then RFC3339
            if let Ok(n) = v.parse::<i64>() {
                return Ok(n);
            }
            chrono::DateTime::parse_from_rfc3339(v)
                .map(|dt| dt.timestamp_millis())
                .map_err(|_| de::Error::custom(format!("invalid timestamp: {v}")))
        }
    }
    deserializer.deserialize_any(TimestampVisitor)
}

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
    pub project_id: String,
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
    Terminated,
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

// ── HexFlo Coordination Types ─────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SwarmInfo {
    pub id: String,
    pub project_id: String,
    pub name: String,
    pub topology: String,
    pub status: String,
    /// Authoritative owner agent ID (ADR-2603241900).
    #[serde(default)]
    pub owner_agent_id: String,
    /// Kept for backward compat — mirrors owner_agent_id.
    #[serde(default)]
    pub created_by: String,
    pub created_at: String,
    pub updated_at: String,
}

/// Returned when a task_assign CAS fails.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "conflict", rename_all = "camelCase")]
pub enum TaskConflict {
    /// Another agent's version was written between our read and assign.
    VersionMismatch { expected: u64, actual: u64 },
    /// Task is no longer pending — already claimed by another agent.
    AlreadyClaimed { by: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SwarmTaskInfo {
    pub id: String,
    pub swarm_id: String,
    pub title: String,
    pub status: String,
    pub agent_id: String,
    pub result: String,
    /// Comma-separated task IDs this task depends on (empty = no deps).
    pub depends_on: String,
    /// Monotonic version for CAS (ADR-2603241900).
    #[serde(default)]
    pub version: u64,
    /// Last agent to claim this task (for conflict messages).
    #[serde(default)]
    pub claimed_by: String,
    pub created_at: String,
    pub completed_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InferenceTaskInfo {
    pub id: String,
    pub workplan_id: String,
    pub task_id: String,
    pub phase: String,
    pub prompt: String,
    pub role: String,
    pub status: String,
    pub agent_id: String,
    pub result: String,
    pub error: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SwarmAgentInfo {
    pub id: String,
    pub swarm_id: String,
    pub name: String,
    pub role: String,
    pub status: String,
    pub worktree_path: String,
    pub last_heartbeat: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CleanupReport {
    pub stale_count: u32,
    pub dead_count: u32,
    pub reclaimed_tasks: u32,
}

// ── Quality Gate & Fix Task Types (Swarm Gate Enforcement) ──

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QualityGateInfo {
    pub id: String,
    pub swarm_id: String,
    pub tier: u32,
    pub gate_type: String,
    pub target_dir: String,
    pub language: String,
    pub status: String,
    pub score: u32,
    pub grade: String,
    pub violations_count: u32,
    pub error_output: String,
    pub iteration: u32,
    pub created_at: String,
    pub completed_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FixTaskInfo {
    pub id: String,
    pub gate_task_id: String,
    pub swarm_id: String,
    pub fix_type: String,
    pub target_file: String,
    pub error_context: String,
    pub model_used: String,
    pub tokens: u64,
    pub cost_usd: String,
    pub status: String,
    pub result: String,
    pub created_at: String,
    pub completed_at: String,
}

// ── Agent Notification Inbox Types (ADR-060) ─────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InboxNotification {
    pub id: u64,
    pub agent_id: String,
    pub priority: u8,
    pub kind: String,
    pub payload: String,
    pub created_at: String,
    pub acknowledged_at: Option<String>,
    pub expired_at: Option<String>,
}

// ── Project Registry Types (ADR-042) ─────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectRegistration {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub root_path: String,
    pub ast_is_stub: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectRecord {
    #[serde(alias = "project_id", alias = "projectId")]
    pub id: String,
    pub name: String,
    #[serde(alias = "path", alias = "root_path")]
    pub root_path: String,
    #[serde(default, deserialize_with = "deserialize_flexible_timestamp")]
    pub registered_at: i64,
    #[serde(default)]
    pub last_push_at: i64,
    #[serde(default)]
    pub health: Option<serde_json::Value>,
    #[serde(default)]
    pub tokens: Option<serde_json::Value>,
    #[serde(default)]
    pub token_files: std::collections::HashMap<String, serde_json::Value>,
    #[serde(default)]
    pub swarm: Option<serde_json::Value>,
    #[serde(default)]
    pub graph: Option<serde_json::Value>,
    #[serde(default, alias = "ast_is_stub")]
    pub ast_is_stub: bool,
}

// ── Instance Coordination Types (ADR-042) ────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstanceRecord {
    pub instance_id: String,
    pub project_id: String,
    pub pid: u32,
    pub session_label: String,
    pub registered_at: String,
    pub last_seen: String,
    pub agent_count: Option<u32>,
    pub active_task_count: Option<u32>,
    pub completed_task_count: Option<u32>,
    pub topology: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstanceHeartbeat {
    pub agent_count: Option<u32>,
    pub active_task_count: Option<u32>,
    pub completed_task_count: Option<u32>,
    pub topology: Option<String>,
}

// ── Worktree Lock Types (ADR-042) ────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorktreeLockRecord {
    pub key: String,
    pub instance_id: String,
    pub project_id: String,
    pub feature: String,
    pub layer: String,
    pub acquired_at: String,
    pub heartbeat_at: String,
    pub ttl_secs: u32,
}

// ── Task Claim Types (ADR-042) ───────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskClaimRecord {
    pub task_id: String,
    pub instance_id: String,
    pub claimed_at: String,
    pub heartbeat_at: String,
}

// ── Unstaged Files Types (ADR-042) ───────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UnstagedFileRecord {
    pub path: String,
    pub status: String,
    pub layer: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UnstagedRecord {
    pub instance_id: String,
    pub project_id: String,
    pub files: Vec<UnstagedFileRecord>,
    pub captured_at: String,
}

// ── Coordination Cleanup (ADR-042) ───────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CoordinationCleanupReport {
    pub instances_removed: usize,
    pub locks_released: usize,
    pub claims_released: usize,
    pub unstaged_removed: usize,
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
    #[serde(rename = "skill_changed")]
    SkillChanged { skill: SkillEntry },
    #[serde(rename = "agent_definition_changed")]
    AgentDefinitionChanged { definition: AgentDefinitionEntry },
    #[serde(rename = "swarm_changed")]
    SwarmChanged { swarm: SwarmInfo },
    #[serde(rename = "swarm_task_changed")]
    SwarmTaskChanged { task: SwarmTaskInfo },
    #[serde(rename = "swarm_agent_changed")]
    SwarmAgentChanged { agent: SwarmAgentInfo },
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
        rate_limited: bool,
        openrouter_cost_usd: f64,
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

    // ── HexFlo Coordination ───────────────────────────
    async fn swarm_init(
        &self,
        id: &str,
        name: &str,
        topology: &str,
        project_id: &str,
        created_by: &str,
    ) -> Result<(), StateError>;
    async fn swarm_complete(&self, id: &str) -> Result<(), StateError>;
    async fn swarm_fail(&self, id: &str, reason: &str) -> Result<(), StateError>;
    async fn swarm_list_active(&self) -> Result<Vec<SwarmInfo>, StateError>;
    async fn swarm_list_failed(&self) -> Result<Vec<SwarmInfo>, StateError>;
    /// Returns all swarms regardless of status, most recent first (capped at `limit`).
    async fn swarm_list_all(&self, limit: usize) -> Result<Vec<SwarmInfo>, StateError>;
    /// Returns all swarms for a project (all statuses — active, completed, failed).
    async fn swarm_list_by_project(&self, project_id: &str) -> Result<Vec<SwarmInfo>, StateError>;
    /// Returns a single swarm by ID regardless of status.
    async fn swarm_get(&self, id: &str) -> Result<Option<SwarmInfo>, StateError>;
    /// Returns the swarm owned by `agent_id` (status=active), if any.
    async fn swarm_owned_by_agent(&self, agent_id: &str) -> Result<Option<SwarmInfo>, StateError>;
    /// Transfer swarm ownership from current owner to `new_owner_agent_id`.
    async fn swarm_transfer(&self, swarm_id: &str, new_owner_agent_id: &str) -> Result<(), StateError>;

    async fn swarm_task_create(
        &self,
        id: &str,
        swarm_id: &str,
        title: &str,
        depends_on: &str,
    ) -> Result<(), StateError>;
    /// Assign a task using CAS. `expected_version` must match current task version.
    /// Pass `None` to skip version check (legacy behaviour).
    /// Returns `Err(StateError::Conflict(_))` on CAS failure.
    async fn swarm_task_assign(
        &self,
        task_id: &str,
        agent_id: &str,
        expected_version: Option<u64>,
    ) -> Result<(), StateError>;
    async fn swarm_task_complete(&self, task_id: &str, result: &str) -> Result<(), StateError>;
    async fn swarm_task_fail(&self, task_id: &str, reason: &str) -> Result<(), StateError>;
    async fn swarm_task_list(&self, swarm_id: Option<&str>) -> Result<Vec<SwarmTaskInfo>, StateError>;

    async fn inference_task_create(&self, id: &str, workplan_id: &str, task_id: &str, phase: &str, prompt: &str, role: &str, created_at: &str) -> Result<(), StateError>;
    async fn inference_task_claim(&self, id: &str, agent_id: &str, updated_at: &str) -> Result<(), StateError>;
    async fn inference_task_complete(&self, id: &str, result: &str, updated_at: &str) -> Result<(), StateError>;
    async fn inference_task_fail(&self, id: &str, error: &str, updated_at: &str) -> Result<(), StateError>;
    async fn inference_task_get(&self, id: &str) -> Result<Option<InferenceTaskInfo>, StateError>;
    async fn inference_task_list_pending(&self) -> Result<Vec<InferenceTaskInfo>, StateError>;

    async fn swarm_agent_register(
        &self,
        id: &str,
        swarm_id: &str,
        name: &str,
        role: &str,
        worktree_path: &str,
    ) -> Result<(), StateError>;
    async fn swarm_agent_heartbeat(&self, id: &str) -> Result<(), StateError>;
    async fn swarm_agent_remove(&self, id: &str) -> Result<(), StateError>;
    async fn swarm_cleanup_stale(
        &self,
        stale_secs: u64,
        dead_secs: u64,
    ) -> Result<CleanupReport, StateError>;

    async fn hexflo_memory_store(
        &self,
        key: &str,
        value: &str,
        scope: &str,
    ) -> Result<(), StateError>;
    async fn hexflo_memory_retrieve(&self, key: &str) -> Result<Option<String>, StateError>;
    async fn hexflo_memory_search(&self, query: &str) -> Result<Vec<(String, String)>, StateError>;
    async fn hexflo_memory_delete(&self, key: &str) -> Result<(), StateError>;

    // ── Quality Gate & Fix Tasks (Swarm Gate Enforcement) ──
    async fn quality_gate_create(
        &self,
        id: &str,
        swarm_id: &str,
        tier: u32,
        gate_type: &str,
        target_dir: &str,
        language: &str,
        iteration: u32,
    ) -> Result<(), StateError>;
    async fn quality_gate_complete(
        &self,
        id: &str,
        status: &str,
        score: u32,
        grade: &str,
        violations_count: u32,
        error_output: &str,
    ) -> Result<(), StateError>;
    async fn quality_gate_list(&self, swarm_id: &str) -> Result<Vec<QualityGateInfo>, StateError>;
    async fn quality_gate_get(&self, id: &str) -> Result<Option<QualityGateInfo>, StateError>;
    async fn fix_task_create(
        &self,
        id: &str,
        gate_task_id: &str,
        swarm_id: &str,
        fix_type: &str,
        target_file: &str,
        error_context: &str,
    ) -> Result<(), StateError>;
    async fn fix_task_complete(
        &self,
        id: &str,
        status: &str,
        result: &str,
        model_used: &str,
        tokens: u64,
        cost_usd: &str,
    ) -> Result<(), StateError>;
    async fn fix_task_list_by_gate(&self, gate_task_id: &str) -> Result<Vec<FixTaskInfo>, StateError>;

    // ── Project Registry (ADR-042) ─────────────────
    async fn project_register(&self, project: ProjectRegistration) -> Result<(), StateError>;
    async fn project_unregister(&self, id: &str) -> Result<bool, StateError>;
    async fn project_get(&self, id: &str) -> Result<Option<ProjectRecord>, StateError>;
    async fn project_list(&self) -> Result<Vec<ProjectRecord>, StateError>;
    async fn project_update_state(
        &self,
        id: &str,
        push_type: &str,
        data: serde_json::Value,
        file_path: Option<&str>,
    ) -> Result<(), StateError>;
    /// Find project by ID, name, or root_path basename.
    async fn project_find(&self, query: &str) -> Result<Option<ProjectRecord>, StateError>;

    // ── Instance Coordination (ADR-042) ──────────────
    async fn instance_register(&self, info: InstanceRecord) -> Result<String, StateError>;
    async fn instance_heartbeat(&self, id: &str, update: InstanceHeartbeat) -> Result<(), StateError>;
    async fn instance_list(&self, project_id: Option<&str>) -> Result<Vec<InstanceRecord>, StateError>;
    async fn instance_remove(&self, id: &str) -> Result<(), StateError>;

    // ── Worktree Locks (ADR-042) ─────────────────────
    async fn worktree_lock_acquire(&self, lock: WorktreeLockRecord) -> Result<bool, StateError>;
    async fn worktree_lock_release(&self, key: &str) -> Result<bool, StateError>;
    async fn worktree_lock_list(&self, project_id: Option<&str>) -> Result<Vec<WorktreeLockRecord>, StateError>;
    async fn worktree_lock_refresh(&self, instance_id: &str, heartbeat_at: &str) -> Result<(), StateError>;
    async fn worktree_lock_evict_expired(&self) -> Result<u32, StateError>;

    // ── Task Claims (ADR-042) ────────────────────────
    async fn task_claim_acquire(&self, claim: TaskClaimRecord) -> Result<bool, StateError>;
    async fn task_claim_release(&self, task_id: &str) -> Result<bool, StateError>;
    async fn task_claim_list(&self, project_id: Option<&str>) -> Result<Vec<TaskClaimRecord>, StateError>;
    async fn task_claim_refresh(&self, instance_id: &str, heartbeat_at: &str) -> Result<(), StateError>;

    // ── Unstaged Files (ADR-042) ─────────────────────
    async fn unstaged_update(&self, instance_id: &str, state: UnstagedRecord) -> Result<(), StateError>;
    async fn unstaged_list(&self, project_id: Option<&str>) -> Result<Vec<UnstagedRecord>, StateError>;
    async fn unstaged_remove(&self, instance_id: &str) -> Result<(), StateError>;

    // ── Coordination Cleanup (ADR-042) ───────────────
    async fn coordination_cleanup_stale(&self, stale_threshold_secs: u64) -> Result<CoordinationCleanupReport, StateError>;

    // ── Unified Agent Registry (ADR-058) ─────────────
    async fn hex_agent_connect(&self, id: &str, name: &str, host: &str, project_id: &str, project_dir: &str, model: &str, session_id: &str, capabilities_json: &str) -> Result<(), StateError>;
    async fn hex_agent_disconnect(&self, id: &str) -> Result<(), StateError>;
    async fn hex_agent_heartbeat(&self, id: &str) -> Result<(), StateError>;
    async fn hex_agent_list(&self) -> Result<Vec<serde_json::Value>, StateError>;
    async fn hex_agent_get(&self, id: &str) -> Result<Option<serde_json::Value>, StateError>;
    async fn hex_agent_evict_dead(&self) -> Result<(), StateError>;
    async fn hex_agent_mark_inactive(&self) -> Result<(), StateError>;

    // ── Agent Notification Inbox (ADR-060) ─────────
    async fn inbox_notify(&self, agent_id: &str, priority: u8, kind: &str, payload: &str) -> Result<(), StateError>;
    async fn inbox_notify_all(&self, project_id: &str, priority: u8, kind: &str, payload: &str) -> Result<(), StateError>;
    async fn inbox_query(&self, agent_id: &str, min_priority: Option<u8>, unacked_only: bool) -> Result<Vec<InboxNotification>, StateError>;
    async fn inbox_acknowledge(&self, notification_id: u64, agent_id: &str) -> Result<(), StateError>;
    async fn inbox_expire(&self, max_age_secs: u64) -> Result<u32, StateError>;

    // ── Neural Lab (architecture search) ──────────────
    async fn neural_lab_config_list(&self, status: Option<&str>) -> Result<Vec<serde_json::Value>, StateError>;
    async fn neural_lab_config_get(&self, id: &str) -> Result<Option<serde_json::Value>, StateError>;
    async fn neural_lab_config_create(&self, args: serde_json::Value) -> Result<serde_json::Value, StateError>;
    async fn neural_lab_layer_specs(&self, config_id: &str) -> Result<Vec<serde_json::Value>, StateError>;
    async fn neural_lab_experiment_list(&self, lineage: Option<&str>, status: Option<&str>) -> Result<Vec<serde_json::Value>, StateError>;
    async fn neural_lab_experiment_get(&self, id: &str) -> Result<Option<serde_json::Value>, StateError>;
    async fn neural_lab_experiment_create(&self, args: serde_json::Value) -> Result<serde_json::Value, StateError>;
    async fn neural_lab_experiment_start(&self, id: &str, gpu_node_id: &str) -> Result<(), StateError>;
    async fn neural_lab_experiment_complete(&self, args: serde_json::Value) -> Result<(), StateError>;
    async fn neural_lab_experiment_fail(&self, id: &str, error_message: &str) -> Result<(), StateError>;
    async fn neural_lab_frontier_get(&self, lineage: &str) -> Result<Option<serde_json::Value>, StateError>;
    async fn neural_lab_strategies_list(&self) -> Result<Vec<serde_json::Value>, StateError>;

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
    /// CAS conflict on task_assign (ADR-2603241900).
    #[error("Conflict: {0}")]
    Conflict(String),
}

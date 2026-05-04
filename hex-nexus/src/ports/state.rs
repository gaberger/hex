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

// ── Workplan Event Log (ADR-2604271000) ──────────────────
//
// The append-only event log that replaces mutable JSON `status` fields as
// the source of truth for workplan progress. The executor emits events at
// every transition; `hex plan project` folds them into a derived view.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WorkplanEventKind {
    Dispatched,
    AgentStopped,
    EvidenceChecked,
    GateRun,
    Demoted,
    ManualMark,
    Migrated,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkplanEventInput {
    pub workplan_id: String,
    pub task_id: String,
    pub kind: WorkplanEventKind,
    /// RFC3339 timestamp of when the transition occurred.
    pub occurred_at: String,
    pub actor: String,
    pub payload: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkplanEvent {
    pub id: String,
    pub workplan_id: String,
    pub task_id: String,
    pub kind: WorkplanEventKind,
    pub occurred_at: String,
    pub actor: String,
    pub payload: serde_json::Value,
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

// ── Focused Sub-Traits (ADR-2604050900 P6) ─────────────
//
// IStatePort was a 100+ method god-trait. These sub-traits let consumers
// depend only on the method groups they actually use. IStatePort remains
// as a super-trait for code that genuinely needs the full surface (e.g.
// composition root, adapter implementations).

/// RL engine: action selection, reward recording, stats.
#[async_trait]
pub trait IRlStatePort: Send + Sync {
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
}

/// Pattern store: store, search, reinforce, decay.
#[async_trait]
pub trait IPatternStatePort: Send + Sync {
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
}

/// Local agent lifecycle: register, status, list, get, remove.
#[async_trait]
pub trait IAgentStatePort: Send + Sync {
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
}

/// Workplan task tracking.
#[async_trait]
pub trait IWorkplanStatePort: Send + Sync {
    async fn workplan_update_task(&self, update: WorkplanTaskUpdate) -> Result<(), StateError>;
    async fn workplan_get_tasks(
        &self,
        workplan_id: &str,
    ) -> Result<Vec<WorkplanTaskUpdate>, StateError>;

    /// Append a workplan transition event (ADR-2604271000 §2).
    ///
    /// Default impl is a no-op so adapters that haven't yet wired the STDB
    /// `workplan_event` reducer (P1.1) keep compiling. The executor still
    /// records every event in an in-process shadow store
    /// (`orchestration::workplan_executor::workplan_event_shadow`) so the
    /// projector and tests can read the sequence without a live STDB.
    async fn workplan_event_append(
        &self,
        input: WorkplanEventInput,
    ) -> Result<String, StateError> {
        let _ = input;
        Ok(String::new())
    }

    /// Read events for a workplan from the underlying STDB log. Default
    /// returns empty so callers fall back to the shadow store until the
    /// reducer + query are wired.
    async fn workplan_events_for(
        &self,
        workplan_id: &str,
    ) -> Result<Vec<WorkplanEvent>, StateError> {
        let _ = workplan_id;
        Ok(Vec::new())
    }
}

/// Chat message send/history.
#[async_trait]
pub trait IChatStatePort: Send + Sync {
    async fn chat_send(&self, message: ChatMessage) -> Result<(), StateError>;
    async fn chat_history(
        &self,
        conversation_id: &str,
        limit: u32,
    ) -> Result<Vec<ChatMessage>, StateError>;
}

/// Skill registry: CRUD + trigger search.
#[async_trait]
pub trait ISkillStatePort: Send + Sync {
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
}

/// Agent definition registry: CRUD + versioning.
#[async_trait]
pub trait IAgentDefStatePort: Send + Sync {
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
}

/// HexFlo swarm lifecycle + task + agent coordination.
#[async_trait]
pub trait ISwarmStatePort: Send + Sync {
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
    async fn swarm_list_all(&self, limit: usize) -> Result<Vec<SwarmInfo>, StateError>;
    async fn swarm_list_by_project(&self, project_id: &str) -> Result<Vec<SwarmInfo>, StateError>;
    async fn swarm_get(&self, id: &str) -> Result<Option<SwarmInfo>, StateError>;
    async fn swarm_owned_by_agent(&self, agent_id: &str) -> Result<Option<SwarmInfo>, StateError>;
    async fn swarm_transfer(&self, swarm_id: &str, new_owner_agent_id: &str) -> Result<(), StateError>;

    async fn swarm_task_create(
        &self,
        id: &str,
        swarm_id: &str,
        title: &str,
        depends_on: &str,
    ) -> Result<(), StateError>;
    async fn swarm_task_assign(
        &self,
        task_id: &str,
        agent_id: &str,
        expected_version: Option<u64>,
    ) -> Result<(), StateError>;
    async fn swarm_task_complete(&self, task_id: &str, result: &str) -> Result<(), StateError>;
    async fn swarm_task_fail(&self, task_id: &str, reason: &str) -> Result<(), StateError>;
    async fn swarm_task_list(&self, swarm_id: Option<&str>) -> Result<Vec<SwarmTaskInfo>, StateError>;

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
}

/// Inference task lifecycle (workplan-driven inference).
#[async_trait]
pub trait IInferenceTaskStatePort: Send + Sync {
    async fn inference_task_create(&self, id: &str, workplan_id: &str, task_id: &str, phase: &str, prompt: &str, role: &str, created_at: &str) -> Result<(), StateError>;
    async fn inference_task_claim(&self, id: &str, agent_id: &str, updated_at: &str) -> Result<(), StateError>;
    async fn inference_task_complete(&self, id: &str, result: &str, updated_at: &str) -> Result<(), StateError>;
    async fn inference_task_fail(&self, id: &str, error: &str, updated_at: &str) -> Result<(), StateError>;
    async fn inference_task_get(&self, id: &str) -> Result<Option<InferenceTaskInfo>, StateError>;
    async fn inference_task_list_pending(&self) -> Result<Vec<InferenceTaskInfo>, StateError>;
}

/// HexFlo key-value memory (scoped: global, per-swarm, per-agent).
#[async_trait]
pub trait IHexFloMemoryStatePort: Send + Sync {
    async fn hexflo_memory_store(
        &self,
        key: &str,
        value: &str,
        scope: &str,
    ) -> Result<(), StateError>;
    async fn hexflo_memory_retrieve(&self, key: &str) -> Result<Option<String>, StateError>;
    async fn hexflo_memory_search(&self, query: &str) -> Result<Vec<(String, String)>, StateError>;
    async fn hexflo_memory_delete(&self, key: &str) -> Result<(), StateError>;
}

/// Quality gate enforcement + fix tasks.
#[async_trait]
pub trait IQualityGateStatePort: Send + Sync {
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
}

/// Project registry (ADR-042).
#[async_trait]
pub trait IProjectStatePort: Send + Sync {
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
    async fn project_find(&self, query: &str) -> Result<Option<ProjectRecord>, StateError>;
}

/// Multi-instance coordination: instances, worktree locks, task claims, unstaged files (ADR-042).
#[async_trait]
pub trait ICoordinationStatePort: Send + Sync {
    async fn instance_register(&self, info: InstanceRecord) -> Result<String, StateError>;
    async fn instance_heartbeat(&self, id: &str, update: InstanceHeartbeat) -> Result<(), StateError>;
    async fn instance_list(&self, project_id: Option<&str>) -> Result<Vec<InstanceRecord>, StateError>;
    async fn instance_remove(&self, id: &str) -> Result<(), StateError>;

    async fn worktree_lock_acquire(&self, lock: WorktreeLockRecord) -> Result<bool, StateError>;
    async fn worktree_lock_release(&self, key: &str) -> Result<bool, StateError>;
    async fn worktree_lock_list(&self, project_id: Option<&str>) -> Result<Vec<WorktreeLockRecord>, StateError>;
    async fn worktree_lock_refresh(&self, instance_id: &str, heartbeat_at: &str) -> Result<(), StateError>;
    async fn worktree_lock_evict_expired(&self) -> Result<u32, StateError>;

    async fn task_claim_acquire(&self, claim: TaskClaimRecord) -> Result<bool, StateError>;
    async fn task_claim_release(&self, task_id: &str) -> Result<bool, StateError>;
    async fn task_claim_list(&self, project_id: Option<&str>) -> Result<Vec<TaskClaimRecord>, StateError>;
    async fn task_claim_refresh(&self, instance_id: &str, heartbeat_at: &str) -> Result<(), StateError>;

    async fn unstaged_update(&self, instance_id: &str, state: UnstagedRecord) -> Result<(), StateError>;
    async fn unstaged_list(&self, project_id: Option<&str>) -> Result<Vec<UnstagedRecord>, StateError>;
    async fn unstaged_remove(&self, instance_id: &str) -> Result<(), StateError>;

    async fn coordination_cleanup_stale(&self, stale_threshold_secs: u64) -> Result<CoordinationCleanupReport, StateError>;
}

/// Unified agent registry (ADR-058).
#[async_trait]
pub trait IHexAgentStatePort: Send + Sync {
    async fn hex_agent_connect(&self, id: &str, name: &str, host: &str, project_id: &str, project_dir: &str, model: &str, session_id: &str, capabilities_json: &str) -> Result<(), StateError>;
    /// Update only the capabilities_json column for an existing agent (ADR-2604130010 P2.1).
    async fn hex_agent_update_capabilities(&self, id: &str, capabilities_json: &str) -> Result<(), StateError>;
    async fn hex_agent_disconnect(&self, id: &str) -> Result<(), StateError>;
    async fn hex_agent_heartbeat(&self, id: &str) -> Result<(), StateError>;
    async fn hex_agent_list(&self) -> Result<Vec<serde_json::Value>, StateError>;
    async fn hex_agent_get(&self, id: &str) -> Result<Option<serde_json::Value>, StateError>;
    async fn hex_agent_evict_dead(&self) -> Result<(), StateError>;
    async fn hex_agent_mark_inactive(&self) -> Result<(), StateError>;

    // ── STDB-as-supervisor (wp-stdb-supervisor P3) ─────────────────────
    /// Query unhandled supervisor_event rows. Returns a list of
    /// (id, kind, pool_id, payload).
    async fn supervisor_event_unhandled(&self) -> Result<Vec<(u64, String, String, String)>, StateError>;
    /// Mark a supervisor_event as handled. handled_by is a free-form
    /// identifier ("nexus-supervisor", "operator", etc.).
    async fn supervisor_event_mark_handled(&self, id: u64, by: &str) -> Result<(), StateError>;
    /// Query the N most recent supervisor_event rows (handled or not) for
    /// dashboard activity-feed surfacing. Returns
    /// (id, ts, kind, pool_id, worker_id, payload, handled).
    async fn supervisor_events_recent(
        &self,
        limit: u32,
    ) -> Result<Vec<(u64, String, String, String, String, String, bool)>, StateError>;
    /// Register a freshly-spawned worker_process row.
    async fn worker_process_register(
        &self,
        id: &str,
        pool_id: &str,
        role: &str,
        host: &str,
        pid: i64,
    ) -> Result<(), StateError>;
    /// Record process exit. exit_reason: "normal" | "crashed" | "killed" | "unknown".
    /// Idempotent — calling twice for the same id is a no-op (the WASM reducer
    /// returns Ok if exited_at is already set).
    async fn worker_process_record_exit(
        &self,
        id: &str,
        exit_reason: &str,
    ) -> Result<(), StateError>;
    /// Look up a pool's role (for the spawn-request fallback path when
    /// the event payload is missing/malformed).
    async fn worker_pool_role(&self, pool_id: &str) -> Result<Option<String>, StateError>;

    // ── Pool CLI / dashboard surface (P4 + P5) ──────────────────────────
    async fn pool_create(
        &self,
        id: &str,
        role: &str,
        desired_count: u32,
        restart_strategy: &str,
        max_restarts: u32,
        max_restart_window_secs: u32,
        paused: bool,
        owner_agent_id: &str,
    ) -> Result<(), StateError>;
    async fn pool_set_paused(&self, id: &str, paused: bool) -> Result<(), StateError>;
    async fn pool_delete(&self, id: &str) -> Result<(), StateError>;
    /// Returns one row per pool with derived alive/exited counts:
    /// (id, role, desired_count, alive_count, exited_count,
    ///  restart_strategy, max_restarts, max_restart_window_secs,
    ///  paused, in_crash_loop)
    async fn pool_status_all(
        &self,
    ) -> Result<Vec<(String, String, u32, u32, u32, String, u32, u32, bool, bool)>, StateError>;
    /// Returns ids of worker_process rows with empty exited_at. Used by the
    /// supervisor subscriber's startup reconciliation pass: if a row says
    /// alive but no watchdog is tracking it (because the previous nexus
    /// process died), assume the worker is dead and mark exited.
    async fn worker_process_orphans(&self) -> Result<Vec<String>, StateError>;
}

/// Agent notification inbox (ADR-060).
#[async_trait]
pub trait IInboxStatePort: Send + Sync {
    async fn inbox_notify(&self, agent_id: &str, priority: u8, kind: &str, payload: &str) -> Result<(), StateError>;
    async fn inbox_notify_all(&self, project_id: &str, priority: u8, kind: &str, payload: &str) -> Result<(), StateError>;
    async fn inbox_query(&self, agent_id: &str, min_priority: Option<u8>, unacked_only: bool) -> Result<Vec<InboxNotification>, StateError>;
    async fn inbox_acknowledge(&self, notification_id: u64, agent_id: &str) -> Result<(), StateError>;
    async fn inbox_expire(&self, max_age_secs: u64) -> Result<u32, StateError>;
}

/// Neural Lab architecture search.
#[async_trait]
pub trait INeuralLabStatePort: Send + Sync {
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
}

// ── Substrate swap-ticket port (ADR-2604261500 P6 / wp-substrate-shadow-promotion P2) ──
//
// Deliberately *not* added to the `IStatePort` super-trait. The substrate
// ADR is about port-by-port modular swapping; expanding the god-trait would
// undermine the very motivation. Consumers (`SpacetimeRuntimeComposition`)
// take `Arc<dyn ISwapTicketStatePort>` directly.
//
// Backed by the `swap_ticket` + `shadow_sample` tables in the
// `hexflo-coordination` STDB module (wp-substrate-shadow-promotion P1).
#[async_trait]
pub trait ISwapTicketStatePort: Send + Sync {
    #[allow(clippy::too_many_arguments)]
    async fn swap_ticket_create(
        &self,
        id: &str,
        project_id: &str,
        port_id: &str,
        incumbent_adapter_id: &str,
        candidate_adapter_id: &str,
        candidate_manifest_json: &str,
        shadow_traffic_fraction: f32,
        shadow_window_seconds: u64,
        success_criteria_json: &str,
        timestamp: &str,
    ) -> Result<(), StateError>;

    async fn swap_ticket_transition(
        &self,
        id: &str,
        new_state: &str,
        timestamp: &str,
    ) -> Result<(), StateError>;

    async fn swap_ticket_set_shadow_started(
        &self,
        id: &str,
        timestamp: &str,
    ) -> Result<(), StateError>;

    /// Update the operator-configurable fields on a non-terminal ticket
    /// (success_criteria_json, shadow_traffic_fraction, shadow_window_seconds).
    /// Called by the propose endpoint after `swap_ticket_create` so the
    /// operator's choices override the propose-time defaults.
    async fn swap_ticket_set_config(
        &self,
        id: &str,
        success_criteria_json: &str,
        shadow_traffic_fraction: f32,
        shadow_window_seconds: u64,
        timestamp: &str,
    ) -> Result<(), StateError>;

    #[allow(clippy::too_many_arguments)]
    async fn shadow_sample_record(
        &self,
        ticket_id: &str,
        call_seq: u64,
        incumbent_adapter_id: &str,
        candidate_adapter_id: &str,
        incumbent_metrics_json: &str,
        candidate_metrics_json: &str,
        agreed: bool,
        reason: &str,
        timestamp: &str,
    ) -> Result<(), StateError>;

    /// Read all swap_tickets currently in `state="shadow"` whose
    /// `shadow_started_at + shadow_window_seconds <= now`. The promotion
    /// judge polls this on its tick.
    async fn shadow_tickets_due(&self, now: &str) -> Result<Vec<SwapTicketRecord>, StateError>;

    /// Read all shadow_samples for a ticket. Used by the promotion judge
    /// to evaluate the success criteria.
    async fn shadow_samples_for(&self, ticket_id: &str) -> Result<Vec<ShadowSampleRecord>, StateError>;

    /// Read all swap_tickets currently in `state="shadow_green"` — i.e.
    /// the judge has decided they're eligible for promotion. The promote
    /// orchestrator polls this on its tick and flips the live binding.
    async fn shadow_green_tickets(&self) -> Result<Vec<SwapTicketRecord>, StateError>;
}

/// Read shape of a `swap_ticket` row.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SwapTicketRecord {
    pub id: String,
    pub project_id: String,
    pub port_id: String,
    pub incumbent_adapter_id: String,
    pub candidate_adapter_id: String,
    pub candidate_manifest_json: String,
    pub state: String,
    pub shadow_traffic_fraction: f32,
    pub shadow_window_seconds: u64,
    pub shadow_started_at: String,
    pub success_criteria_json: String,
    pub created_at: String,
    pub updated_at: String,
}

/// Read shape of a `shadow_sample` row.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ShadowSampleRecord {
    pub id: u64,
    pub ticket_id: String,
    pub call_seq: u64,
    pub incumbent_adapter_id: String,
    pub candidate_adapter_id: String,
    pub incumbent_metrics_json: String,
    pub candidate_metrics_json: String,
    pub agreed: bool,
    pub reason: String,
    pub recorded_at: String,
}

// ── The Unified Super-Trait ─────────────────────────────
//
// Existing code that takes `Arc<dyn IStatePort>` continues to work.
// New code should prefer narrow sub-traits where possible.

/// Unified state port — extends all focused sub-traits.
///
/// Implementation: `SpacetimeStateAdapter` (the only implementation —
/// SQLite was removed per the STDB-only directive). Real-time subscriptions
/// are delivered via STDB's reactive channel.
#[async_trait]
pub trait IStatePort:
    IRlStatePort
    + IPatternStatePort
    + IAgentStatePort
    + IWorkplanStatePort
    + IChatStatePort
    + ISkillStatePort
    + IAgentDefStatePort
    + ISwarmStatePort
    + IInferenceTaskStatePort
    + IHexFloMemoryStatePort
    + IQualityGateStatePort
    + IProjectStatePort
    + ICoordinationStatePort
    + IHexAgentStatePort
    + IInboxStatePort
    + INeuralLabStatePort
{
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

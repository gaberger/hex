//! Coordination port — contract for distributed agent coordination.
//!
//! Implemented by SpacetimeDB adapters. Covers file locking,
//! architecture enforcement, swarm management, and memory.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::domain::agents::AgentStatus;
use crate::domain::workplan::TaskStatus;

/// File lock types for multi-agent coordination.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LockType {
    /// Only one agent can hold this lock.
    Exclusive,
    /// Multiple agents can read simultaneously.
    SharedRead,
}

/// A file lock claim.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileLock {
    pub file_path: String,
    pub agent_id: String,
    pub lock_type: LockType,
    pub acquired_at: String,
    pub expires_at: String,
    pub worktree: Option<String>,
}

/// Result of a boundary validation check.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WriteValidation {
    pub validation_id: String,
    pub agent_id: String,
    pub file_path: String,
    pub verdict: Verdict,
    pub violations: Vec<String>,
}

/// Verdict from the architecture enforcer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    Approved,
    Rejected,
}

/// Swarm metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SwarmInfo {
    pub id: String,
    pub project_id: String,
    pub name: String,
    pub topology: String,
    pub status: String,
    pub created_at: String,
    pub updated_at: String,
}

/// A task within a swarm.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SwarmTask {
    pub id: String,
    pub swarm_id: String,
    pub title: String,
    pub status: TaskStatus,
    pub agent_id: Option<String>,
    pub result: Option<String>,
    pub created_at: String,
    pub completed_at: Option<String>,
}

/// An agent within a swarm.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SwarmAgent {
    pub id: String,
    pub swarm_id: String,
    pub name: String,
    pub role: String,
    pub status: AgentStatus,
    pub worktree_path: Option<String>,
    pub last_heartbeat: String,
}

/// The coordination port — SpacetimeDB implements this.
#[async_trait]
pub trait ICoordinationPort: Send + Sync {
    // ── File locking ──────────────────────────────────────
    async fn acquire_file_lock(
        &self,
        file_path: &str,
        agent_id: &str,
        lock_type: LockType,
    ) -> Result<FileLock, CoordinationError>;

    async fn release_file_lock(
        &self,
        file_path: &str,
        agent_id: &str,
    ) -> Result<(), CoordinationError>;

    // ── Architecture enforcement ──────────────────────────
    async fn validate_write(
        &self,
        agent_id: &str,
        file_path: &str,
        proposed_imports: &[String],
    ) -> Result<WriteValidation, CoordinationError>;

    // ── Swarm management ──────────────────────────────────
    async fn swarm_init(
        &self,
        name: &str,
        topology: &str,
    ) -> Result<SwarmInfo, CoordinationError>;

    async fn swarm_status(&self) -> Result<Vec<SwarmInfo>, CoordinationError>;

    async fn task_create(
        &self,
        swarm_id: &str,
        title: &str,
    ) -> Result<SwarmTask, CoordinationError>;

    async fn task_complete(
        &self,
        task_id: &str,
        result: &str,
    ) -> Result<(), CoordinationError>;

    // ── Memory (key-value) ────────────────────────────────
    async fn memory_store(
        &self,
        key: &str,
        value: &str,
        scope: Option<&str>,
    ) -> Result<(), CoordinationError>;

    async fn memory_retrieve(
        &self,
        key: &str,
    ) -> Result<Option<String>, CoordinationError>;

    async fn memory_search(
        &self,
        query: &str,
    ) -> Result<Vec<(String, String)>, CoordinationError>;

    // ── Agent heartbeat ───────────────────────────────────
    async fn heartbeat(
        &self,
        agent_id: &str,
        status: &AgentStatus,
        turn_count: u32,
        token_usage: u64,
    ) -> Result<(), CoordinationError>;
}

#[derive(Debug, thiserror::Error)]
pub enum CoordinationError {
    #[error("File lock conflict: {file_path} held by {held_by}")]
    LockConflict {
        file_path: String,
        held_by: String,
    },
    #[error("Boundary violation: {0}")]
    BoundaryViolation(String),
    #[error("Swarm not found: {0}")]
    SwarmNotFound(String),
    #[error("Task not found: {0}")]
    TaskNotFound(String),
    #[error("Connection error: {0}")]
    Connection(String),
}

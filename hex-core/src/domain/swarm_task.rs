//! Shared types for HexFlo swarm task lifecycle.
//!
//! Both `hex-agent` (sender) and `hex-nexus` (receiver) import these types,
//! making mismatched status strings a compile error rather than a runtime bug.
//! See ADR-2603311000.

use serde::{Deserialize, Serialize};

/// Typed status for a swarm task completion PATCH.
///
/// Serializes as lowercase strings (`"completed"`, `"failed"`) to stay
/// compatible with the existing nexus PATCH `/api/swarms/tasks/:id` handler.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SwarmTaskStatus {
    Pending,
    InProgress,
    Completed,
    Failed,
}

impl SwarmTaskStatus {
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Completed | Self::Failed)
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::InProgress => "in_progress",
            Self::Completed => "completed",
            Self::Failed => "failed",
        }
    }
}

impl std::fmt::Display for SwarmTaskStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl TryFrom<&str> for SwarmTaskStatus {
    type Error = String;
    fn try_from(s: &str) -> Result<Self, Self::Error> {
        match s {
            "pending" => Ok(Self::Pending),
            "in_progress" | "in-progress" => Ok(Self::InProgress),
            "completed" => Ok(Self::Completed),
            "failed" => Ok(Self::Failed),
            other => Err(format!("unknown SwarmTaskStatus: {other}")),
        }
    }
}

/// Body sent by `hex-agent` when completing or failing a swarm task.
///
/// Used for `PATCH /api/swarms/tasks/:id` and `PATCH /api/hexflo/tasks/:id`.
/// Both sides share this type — any field rename is a compile error.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SwarmTaskCompletion {
    /// Terminal status — must be `completed` or `failed`.
    pub status: SwarmTaskStatus,
    /// Human-readable result or error message (max ~2000 chars recommended).
    pub result: String,
    /// Agent that performed the work — stored for audit trail.
    pub agent_id: String,
}

impl SwarmTaskCompletion {
    pub fn success(result: impl Into<String>, agent_id: impl Into<String>) -> Self {
        Self {
            status: SwarmTaskStatus::Completed,
            result: result.into(),
            agent_id: agent_id.into(),
        }
    }

    pub fn failure(reason: impl Into<String>, agent_id: impl Into<String>) -> Self {
        Self {
            status: SwarmTaskStatus::Failed,
            result: reason.into(),
            agent_id: agent_id.into(),
        }
    }
}

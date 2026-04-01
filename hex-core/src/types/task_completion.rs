use serde::{Deserialize, Serialize};

use crate::domain::workplan::TaskStatus;

/// HTTP body for task completion/status-update endpoints.
///
/// Shared by hex-nexus REST handlers and hex-cli to avoid duplicate definitions.
/// Uses the canonical [`TaskStatus`] from `domain::workplan` — no parallel enum.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskCompletionBody {
    pub task_id: String,
    pub status: TaskStatus,
    pub result: Option<String>,
    pub error: Option<String>,
    pub agent_id: Option<String>,
}

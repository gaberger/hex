//! Shared wire types used across hex-cli, hex-nexus, and hex-agent.
//!
//! These types define the JSON contracts for inter-component communication.

use serde::{Deserialize, Serialize};

use crate::domain::workplan::TaskStatus;

/// JSON body sent by agents to complete a HexFlo task.
///
/// Used by: hex-agent task_executor, stdb_task_poller, hex-nexus swarms route.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskCompletionBody {
    pub status: TaskStatus,
    pub result: String,
    pub agent_id: String,
}

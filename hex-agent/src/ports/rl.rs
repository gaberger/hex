use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// Discretized state for the RL engine — describes the current task context.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RlState {
    pub task_type: String,
    pub codebase_size: u64,
    pub agent_count: u8,
    pub token_usage: u64,
}

/// Action selected by the RL engine.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RlAction {
    pub action: String,
    pub state_key: String,
}

/// Context packing strategy derived from an RL action.
#[derive(Debug, Clone, PartialEq)]
pub enum ContextStrategy {
    /// Aggressive: maximize context usage, evict less, larger tool results
    Aggressive,
    /// Balanced: default partition ratios
    Balanced,
    /// Conservative: smaller windows, summarize early, preserve token budget
    Conservative,
}

impl ContextStrategy {
    pub fn from_action(action: &str) -> Self {
        match action {
            "context:aggressive" => Self::Aggressive,
            "context:conservative" => Self::Conservative,
            _ => Self::Balanced,
        }
    }

    /// Multiplier for the history partition of the token budget.
    pub fn history_multiplier(&self) -> f32 {
        match self {
            Self::Aggressive => 1.3,
            Self::Balanced => 1.0,
            Self::Conservative => 0.7,
        }
    }

    /// Multiplier for the tool results partition.
    pub fn tool_multiplier(&self) -> f32 {
        match self {
            Self::Aggressive => 1.2,
            Self::Balanced => 1.0,
            Self::Conservative => 0.8,
        }
    }
}

/// Reward signal sent back to the RL engine after a task.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RlReward {
    pub state_key: String,
    pub action: String,
    pub reward: f64,
    pub next_state_key: String,
}

/// Port for querying the RL engine in hex-hub.
#[async_trait]
pub trait RlPort: Send + Sync {
    /// Query the RL engine for the optimal action given current state.
    async fn select_action(&self, state: &RlState) -> Result<RlAction, RlError>;

    /// Report a reward to the RL engine after task completion.
    async fn report_reward(&self, reward: &RlReward) -> Result<(), RlError>;
}

#[derive(Debug, thiserror::Error)]
pub enum RlError {
    #[error("RL service unavailable: {0}")]
    Unavailable(String),
    #[error("RL request failed: {0}")]
    RequestFailed(String),
}

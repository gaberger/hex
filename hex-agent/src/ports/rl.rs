use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// Discretized state for the RL engine — describes the current task context.
#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct RlState {
    pub task_type: String,
    pub codebase_size: u64,
    pub agent_count: u8,
    pub token_usage: u64,
    /// Whether the last API request was rate-limited (HTTP 429).
    #[serde(default)]
    pub rate_limited: bool,
    /// Cumulative retry count for this session.
    #[serde(default)]
    pub retry_count: u8,
    /// Model currently in use (e.g. "claude-sonnet-4-6").
    #[serde(default = "default_model")]
    pub current_model: String,
}

#[allow(dead_code)] // used by serde(default) at runtime
fn default_model() -> String {
    ModelSelection::Sonnet.model_id().to_string()
}

/// Model selection derived from an RL action — which LLM to route to.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[derive(Default)]
pub enum ModelSelection {
    /// claude-opus — highest quality
    Opus,
    /// claude-sonnet — balanced (default)
    #[default]
    Sonnet,
    /// claude-haiku — fast/cheap
    Haiku,
    /// MiniMax M2.5 — near-Opus quality, 10-50x cheaper
    MiniMax,
    /// MiniMax M2.5-Lightning — 2x speed, slightly higher output cost
    MiniMaxFast,
    /// OpenRouter model — dynamic ID (e.g. "meta-llama/llama-4-maverick")
    OpenRouter(String),
    /// ollama/vllm — no rate limits
    Local,
}


impl ModelSelection {
    /// Parse a model directive from the RL action string segment.
    /// E.g. "model:opus" -> Opus, "model:local" -> Local.
    pub fn from_action(action: &str) -> Self {
        match action {
            "model:opus" => Self::Opus,
            "model:haiku" => Self::Haiku,
            "model:minimax" => Self::MiniMax,
            "model:minimax_fast" => Self::MiniMaxFast,
            "model:local" => Self::Local,
            s if s.starts_with("model:openrouter:") => {
                Self::OpenRouter(s.trim_start_matches("model:openrouter:").to_string())
            }
            _ => Self::Sonnet,
        }
    }

    /// Returns the actual API model identifier.
    pub fn model_id(&self) -> &str {
        match self {
            Self::Opus => "claude-opus-4-6",
            Self::Sonnet => "claude-sonnet-4-6",
            Self::Haiku => "claude-haiku-4-5-20251001",
            Self::MiniMax => "MiniMax-M2.7",
            Self::MiniMaxFast => "MiniMax-M1",
            Self::OpenRouter(ref id) => id.as_str(),
            Self::Local => "local",
        }
    }

    /// Whether this selection routes to a local inference engine.
    pub fn is_local(&self) -> bool {
        matches!(self, Self::Local)
    }

    /// Whether this selection routes to an OpenAI-compatible provider (not Anthropic).
    pub fn is_openai_compat(&self) -> bool {
        matches!(self, Self::MiniMax | Self::MiniMaxFast | Self::OpenRouter(_) | Self::Local)
    }
}

/// Action selected by the RL engine.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RlAction {
    pub action: String,
    pub state_key: String,
    /// Model selection parsed from the compound action string.
    #[serde(skip)]
    pub model: ModelSelection,
    /// Context strategy parsed from the compound action string.
    #[serde(skip)]
    pub context_strategy: ContextStrategy,
}

impl RlAction {
    /// Parse a compound action string into model + context selections.
    ///
    /// Format: "model:<variant>|context:<variant>" (pipe-delimited).
    /// If no pipe is present the whole string is treated as a context-only
    /// directive with the default model (Sonnet).
    pub fn parse(action: String, state_key: String) -> Self {
        let (model, context_strategy) = Self::parse_compound(&action);
        Self {
            action,
            state_key,
            model,
            context_strategy,
        }
    }

    fn parse_compound(action: &str) -> (ModelSelection, ContextStrategy) {
        let mut model = ModelSelection::default();
        let mut strategy = ContextStrategy::Balanced;

        for segment in action.split('|') {
            let segment = segment.trim();
            if segment.starts_with("model:") {
                model = ModelSelection::from_action(segment);
            } else if segment.starts_with("context:") {
                strategy = ContextStrategy::from_action(segment);
            }
        }

        (model, strategy)
    }
}

/// Context packing strategy derived from an RL action.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[derive(Default)]
pub enum ContextStrategy {
    /// Aggressive: maximize context usage, evict less, larger tool results
    Aggressive,
    /// Balanced: default partition ratios
    #[default]
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
#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct RlReward {
    pub state_key: String,
    pub action: String,
    pub reward: f64,
    pub next_state_key: String,
    /// Whether this turn was rate-limited at any point.
    #[serde(default)]
    pub rate_limited: bool,
    /// Which model was actually used for inference.
    #[serde(default)]
    pub model_used: String,
    /// End-to-end response latency in milliseconds.
    #[serde(default)]
    pub latency_ms: u64,
    /// Actual cost in USD from OpenRouter (0.0 if not an OpenRouter model or unknown).
    /// When > 0, the RL engine uses this for cost-efficiency reward instead of token estimates.
    #[serde(default)]
    pub openrouter_cost_usd: f64,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_selection_from_action() {
        assert_eq!(ModelSelection::from_action("model:opus"), ModelSelection::Opus);
        assert_eq!(ModelSelection::from_action("model:sonnet"), ModelSelection::Sonnet);
        assert_eq!(ModelSelection::from_action("model:haiku"), ModelSelection::Haiku);
        assert_eq!(ModelSelection::from_action("model:local"), ModelSelection::Local);
        assert_eq!(
            ModelSelection::from_action("model:openrouter:meta-llama/llama-4-maverick"),
            ModelSelection::OpenRouter("meta-llama/llama-4-maverick".to_string())
        );
        assert_eq!(ModelSelection::from_action("unknown"), ModelSelection::Sonnet);
    }

    #[test]
    fn model_selection_model_ids() {
        assert_eq!(ModelSelection::Opus.model_id(), "claude-opus-4-6");
        assert_eq!(ModelSelection::Sonnet.model_id(), "claude-sonnet-4-6");
        assert_eq!(ModelSelection::Haiku.model_id(), "claude-haiku-4-5-20251001");
        assert_eq!(ModelSelection::Local.model_id(), "local");
    }

    #[test]
    fn model_selection_is_local() {
        assert!(!ModelSelection::Opus.is_local());
        assert!(!ModelSelection::Sonnet.is_local());
        assert!(!ModelSelection::Haiku.is_local());
        assert!(ModelSelection::Local.is_local());
    }

    #[test]
    fn rl_action_parse_compound() {
        let action = RlAction::parse(
            "model:opus|context:aggressive".to_string(),
            "key1".to_string(),
        );
        assert_eq!(action.model, ModelSelection::Opus);
        assert_eq!(action.context_strategy, ContextStrategy::Aggressive);
        assert_eq!(action.action, "model:opus|context:aggressive");
        assert_eq!(action.state_key, "key1");
    }

    #[test]
    fn rl_action_parse_context_only_defaults_to_sonnet() {
        let action = RlAction::parse(
            "context:conservative".to_string(),
            "key2".to_string(),
        );
        assert_eq!(action.model, ModelSelection::Sonnet);
        assert_eq!(action.context_strategy, ContextStrategy::Conservative);
    }

    #[test]
    fn rl_action_parse_model_only_defaults_to_balanced() {
        let action = RlAction::parse(
            "model:haiku".to_string(),
            "key3".to_string(),
        );
        assert_eq!(action.model, ModelSelection::Haiku);
        assert_eq!(action.context_strategy, ContextStrategy::Balanced);
    }

    #[test]
    fn rl_action_parse_unknown_string() {
        let action = RlAction::parse(
            "something_unknown".to_string(),
            "key4".to_string(),
        );
        assert_eq!(action.model, ModelSelection::Sonnet);
        assert_eq!(action.context_strategy, ContextStrategy::Balanced);
    }

    #[test]
    fn rl_action_parse_reversed_order() {
        let action = RlAction::parse(
            "context:conservative|model:local".to_string(),
            "key5".to_string(),
        );
        assert_eq!(action.model, ModelSelection::Local);
        assert_eq!(action.context_strategy, ContextStrategy::Conservative);
    }

    #[test]
    fn context_strategy_from_action_unchanged() {
        assert_eq!(ContextStrategy::from_action("context:aggressive"), ContextStrategy::Aggressive);
        assert_eq!(ContextStrategy::from_action("context:balanced"), ContextStrategy::Balanced);
        assert_eq!(ContextStrategy::from_action("context:conservative"), ContextStrategy::Conservative);
        assert_eq!(ContextStrategy::from_action("other"), ContextStrategy::Balanced);
    }

    #[test]
    fn rl_state_new_fields_default() {
        let state = RlState {
            task_type: "test".to_string(),
            codebase_size: 100,
            agent_count: 1,
            token_usage: 500,
            rate_limited: false,
            retry_count: 0,
            current_model: default_model(),
        };
        assert!(!state.rate_limited);
        assert_eq!(state.retry_count, 0);
        assert_eq!(state.current_model, "claude-sonnet-4-6");
    }

    #[test]
    fn rl_reward_new_fields() {
        let reward = RlReward {
            state_key: "k".to_string(),
            action: "a".to_string(),
            reward: 0.8,
            next_state_key: "nk".to_string(),
            rate_limited: true,
            model_used: "claude-opus-4-6".to_string(),
            latency_ms: 1234,
            openrouter_cost_usd: 0.0,
        };
        assert!(reward.rate_limited);
        assert_eq!(reward.model_used, "claude-opus-4-6");
        assert_eq!(reward.latency_ms, 1234);
        assert_eq!(reward.openrouter_cost_usd, 0.0);
    }

    #[test]
    fn rl_reward_openrouter_cost() {
        let reward = RlReward {
            state_key: "k".to_string(),
            action: "model:openrouter:meta-llama/llama-4-maverick".to_string(),
            reward: 0.5,
            next_state_key: "nk".to_string(),
            rate_limited: false,
            model_used: "meta-llama/llama-4-maverick".to_string(),
            latency_ms: 800,
            openrouter_cost_usd: 0.0042,
        };
        assert!(!reward.rate_limited);
        assert!(reward.openrouter_cost_usd > 0.0);
    }
}

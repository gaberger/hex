//! RL-driven model selection for `hex dev` pipeline phases.
//!
//! Each pipeline phase (specs, plan, code-gen, validation) maps to a [`TaskType`]
//! which the RL engine uses to recommend the best OpenRouter model. When the RL
//! engine has no data yet, hardcoded defaults are used.
//!
//! After each phase completes, [`ModelSelector::report_outcome`] feeds the reward
//! signal back so the RL engine learns which models work best per task type.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use tracing::{debug, warn};

use crate::nexus_client::NexusClient;

// ── Task type (mirrors hex-agent's port type) ────────────────────────────

/// The type of task being performed — determines RL model recommendation.
///
/// This is a local copy of `hex_agent::ports::inference_client::TaskType` so
/// that hex-cli does not need a crate dependency on hex-agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskType {
    /// Chain-of-thought reasoning (planning, architecture decisions)
    Reasoning,
    /// JSON/structured output generation
    StructuredOutput,
    /// Writing new code
    CodeGeneration,
    /// Editing existing code (diffs, refactors)
    CodeEdit,
    /// General-purpose chat/assistant
    General,
}

impl std::fmt::Display for TaskType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Reasoning => write!(f, "reasoning"),
            Self::StructuredOutput => write!(f, "structured_output"),
            Self::CodeGeneration => write!(f, "code_generation"),
            Self::CodeEdit => write!(f, "code_edit"),
            Self::General => write!(f, "general"),
        }
    }
}

// ── Defaults ─────────────────────────────────────────────────────────────

/// Default model: OpenRouter auto-router picks the best model per request.
/// Falls back to task-specific models if auto isn't available.
fn default_model_for(task_type: TaskType) -> &'static str {
    // openrouter/auto lets OpenRouter pick the best model for each request
    // based on prompt content, context length, and cost optimization.
    // See: https://openrouter.ai/docs/guides/routing/routers/auto-router
    match task_type {
        TaskType::Reasoning => "openrouter/auto",
        TaskType::StructuredOutput => "openrouter/auto",
        TaskType::CodeGeneration => "openrouter/auto",
        TaskType::CodeEdit => "openrouter/auto",
        TaskType::General => "openrouter/auto",
    }
}

/// Free-tier fallback: `openrouter/free` routes to the best free model.
/// Used when paid credits are exhausted (402/insufficient credits).
pub fn free_fallback_for(_task_type: TaskType) -> &'static str {
    // openrouter/free lets OpenRouter pick the best FREE model per request.
    // No per-task-type selection needed — OpenRouter handles it.
    "openrouter/free"
}

/// Ordered fallback chain: auto (paid) → free → specific free models → ollama.
pub fn fallback_chain_for(_task_type: TaskType) -> Vec<&'static str> {
    vec![
        "openrouter/auto",  // Best paid model (OpenRouter picks)
        "openrouter/free",  // Best free model (OpenRouter picks)
    ]
}

// ── RL response types ────────────────────────────────────────────────────

/// Response from `POST /api/rl/action`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RlActionResponse {
    action: String,
    state_key: String,
}

// ── ModelSelector ────────────────────────────────────────────────────────

/// Selects the best model for a given task type using the RL engine,
/// with hardcoded fallbacks when the engine is unavailable or cold.
pub struct ModelSelector {
    client: NexusClient,
}

/// Result of a model selection — includes metadata for reward reporting.
#[derive(Debug, Clone)]
pub struct SelectedModel {
    /// The model identifier to use for inference.
    pub model_id: String,
    /// The RL state key (needed for reward reporting). `None` if RL was bypassed.
    pub state_key: Option<String>,
    /// The raw RL action string. `None` if RL was bypassed.
    pub action: Option<String>,
    /// Whether this was an RL recommendation or a fallback/override.
    pub source: SelectionSource,
}

/// How the model was selected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SelectionSource {
    /// RL engine recommended this model.
    RlEngine,
    /// Hardcoded default (RL had no data or was unavailable).
    Default,
    /// User explicitly set `--model`.
    UserOverride,
    /// Filtered to a specific provider preference.
    ProviderFiltered,
}

impl std::fmt::Display for SelectionSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::RlEngine => write!(f, "rl-engine"),
            Self::Default => write!(f, "default"),
            Self::UserOverride => write!(f, "user-override"),
            Self::ProviderFiltered => write!(f, "provider-filtered"),
        }
    }
}

impl ModelSelector {
    /// Create a new selector that communicates with hex-nexus at the given base URL.
    pub fn new(nexus_base_url: &str) -> Self {
        Self {
            client: NexusClient::new(nexus_base_url.to_string()),
        }
    }

    /// Create a selector using the standard nexus URL resolution (env / port file / default).
    pub fn from_env() -> Self {
        Self {
            client: NexusClient::from_env(),
        }
    }

    /// Select the best model for a pipeline phase.
    ///
    /// # Arguments
    /// * `task_type` — the kind of work this phase performs
    /// * `model_override` — if `Some`, skip RL entirely and use this model
    /// * `provider_preference` — if `Some`, filter to models from this provider (e.g. "ollama")
    pub async fn select_model(
        &self,
        task_type: TaskType,
        model_override: Option<&str>,
        provider_preference: Option<&str>,
    ) -> Result<SelectedModel> {
        // Fast path: user explicitly chose a model — skip RL entirely.
        if let Some(model) = model_override {
            debug!(model, %task_type, "model override — skipping RL");
            return Ok(SelectedModel {
                model_id: model.to_string(),
                state_key: None,
                action: None,
                source: SelectionSource::UserOverride,
            });
        }

        // Provider preference: use hardcoded default filtered to that provider.
        if let Some(provider) = provider_preference {
            let default = default_model_for(task_type);
            let model_id = if default.starts_with(&format!("{}-", provider)) {
                default.to_string()
            } else {
                // Best-effort: prefix the provider to the default model's suffix.
                // In practice, the caller would need a provider-specific mapping,
                // but for now we just return the default and log a warning.
                warn!(
                    %provider, %task_type,
                    "provider preference set but default model is not from that provider; using default"
                );
                default.to_string()
            };
            return Ok(SelectedModel {
                model_id,
                state_key: None,
                action: None,
                source: SelectionSource::ProviderFiltered,
            });
        }

        // Ask the RL engine for a recommendation.
        match self.query_rl_engine(task_type).await {
            Ok(selected) => {
                debug!(
                    model = %selected.model_id,
                    %task_type,
                    source = %selected.source,
                    "RL engine recommended model"
                );
                Ok(selected)
            }
            Err(e) => {
                warn!(%task_type, error = %e, "RL engine unavailable — using default model");
                Ok(SelectedModel {
                    model_id: default_model_for(task_type).to_string(),
                    state_key: None,
                    action: None,
                    source: SelectionSource::Default,
                })
            }
        }
    }

    /// Query the RL engine via `POST /api/rl/action`.
    async fn query_rl_engine(&self, task_type: TaskType) -> Result<SelectedModel> {
        let body = serde_json::json!({
            "state": {
                "taskType": task_type.to_string(),
                "codebaseSize": 0_u64,
                "agentCount": 1_u8,
                "tokenUsage": 0_u64
            }
        });

        let resp: serde_json::Value = self
            .client
            .post("/api/rl/action", &body)
            .await
            .context("POST /api/rl/action failed")?;

        let rl: RlActionResponse = serde_json::from_value(resp)
            .context("unexpected shape from /api/rl/action")?;

        // Parse compound action string to extract the model directive.
        let model_id = extract_model_from_action(&rl.action, task_type);

        Ok(SelectedModel {
            model_id,
            state_key: Some(rl.state_key),
            action: Some(rl.action),
            source: SelectionSource::RlEngine,
        })
    }

    /// Report the outcome of a pipeline phase back to the RL engine.
    ///
    /// This feeds the reward signal so the engine learns which models work
    /// best for each task type. Call this after every phase, even failures.
    ///
    /// # Arguments
    /// * `selected` — the model selection returned by [`select_model`]
    /// * `task_type` — the task type (used to build the next state key)
    /// * `success` — whether the phase succeeded (hex analyze passed, no violations)
    /// * `cost_usd` — actual cost from OpenRouter (0.0 if unknown)
    /// * `duration_ms` — wall-clock duration of the phase
    pub async fn report_outcome(
        &self,
        selected: &SelectedModel,
        task_type: TaskType,
        success: bool,
        cost_usd: f64,
        duration_ms: u64,
    ) -> Result<()> {
        // Only report to RL if we actually used an RL recommendation.
        let (state_key, action) = match (&selected.state_key, &selected.action) {
            (Some(sk), Some(act)) => (sk.clone(), act.clone()),
            _ => {
                debug!(
                    source = %selected.source,
                    "skipping RL reward report — model was not RL-selected"
                );
                return Ok(());
            }
        };

        // Reward formula:
        //   base    = 1.0 on success, -0.5 on failure
        //   cost    = -cost_usd * 10 (penalize expensive models)
        //   speed   = bonus for fast responses (< 5s = +0.2, < 15s = +0.1)
        let base = if success { 1.0 } else { -0.5 };
        let cost_penalty = -cost_usd * 10.0;
        let speed_bonus = if duration_ms < 5_000 {
            0.2
        } else if duration_ms < 15_000 {
            0.1
        } else {
            0.0
        };
        let reward = (base + cost_penalty + speed_bonus).clamp(-1.0, 1.0);

        let next_state_key = format!(
            "{}_{}_{}",
            task_type,
            if success { "ok" } else { "fail" },
            duration_ms / 1000
        );

        let body = serde_json::json!({
            "stateKey": state_key,
            "action": action,
            "reward": reward,
            "nextStateKey": next_state_key,
            "rateLimited": false,
            "openrouterCostUsd": cost_usd
        });

        self.client
            .post("/api/rl/reward", &body)
            .await
            .context("POST /api/rl/reward failed")?;

        debug!(
            model = %selected.model_id,
            %task_type,
            %reward,
            %success,
            cost_usd,
            duration_ms,
            "reported RL reward"
        );

        Ok(())
    }
}

/// Extract a model ID from a compound RL action string.
///
/// Action format: `"model:<variant>|context:<variant>"` (pipe-delimited).
/// If the action contains `model:openrouter:<id>`, we return the OpenRouter
/// model ID prefixed with `openrouter-`. Otherwise, fall back to the
/// hardcoded default for the task type.
fn extract_model_from_action(action: &str, task_type: TaskType) -> String {
    for segment in action.split('|') {
        let segment = segment.trim();
        if segment.starts_with("model:openrouter:") {
            let or_id = segment.trim_start_matches("model:openrouter:");
            // Convert slash-based OpenRouter IDs to dash-based hex IDs:
            // "meta-llama/llama-4-maverick" -> "meta-llama/llama-4-maverick"
            let hex_id = format!("openrouter-{}", or_id.replace('/', "-"));
            return hex_id;
        }
        if segment.starts_with("model:") {
            // Non-OpenRouter model from RL — map known variants.
            return match segment {
                "model:opus" => "claude-opus-4-6".to_string(),
                "model:sonnet" => "claude-sonnet-4-6".to_string(),
                "model:haiku" => "claude-haiku-4-5-20251001".to_string(),
                "model:minimax" => "MiniMax-M2.7".to_string(),
                "model:minimax_fast" => "MiniMax-M1".to_string(),
                "model:local" => "local".to_string(),
                _ => default_model_for(task_type).to_string(),
            };
        }
    }
    // No model directive in action — use default.
    default_model_for(task_type).to_string()
}

// ── Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_models_are_openrouter() {
        for task_type in [
            TaskType::Reasoning,
            TaskType::StructuredOutput,
            TaskType::CodeGeneration,
            TaskType::CodeEdit,
            TaskType::General,
        ] {
            let model = default_model_for(task_type);
            assert!(
                model.starts_with("openrouter-"),
                "default for {:?} should be OpenRouter, got: {}",
                task_type,
                model
            );
        }
    }

    #[test]
    fn extract_openrouter_model() {
        let action = "model:openrouter:meta-llama/llama-4-maverick|context:balanced";
        let model = extract_model_from_action(action, TaskType::CodeGeneration);
        assert_eq!(model, "meta-llama/llama-4-maverick");
    }

    #[test]
    fn extract_anthropic_model() {
        let action = "model:opus|context:aggressive";
        let model = extract_model_from_action(action, TaskType::Reasoning);
        assert_eq!(model, "claude-opus-4-6");
    }

    #[test]
    fn extract_model_fallback_to_default() {
        let action = "context:conservative";
        let model = extract_model_from_action(action, TaskType::Reasoning);
        assert_eq!(model, "deepseek/deepseek-r1");
    }

    #[test]
    fn extract_model_unknown_variant_falls_back() {
        let action = "model:unknown_thing|context:balanced";
        let model = extract_model_from_action(action, TaskType::CodeEdit);
        assert_eq!(model, "deepseek/deepseek-r1");
    }

    #[test]
    fn task_type_display() {
        assert_eq!(TaskType::Reasoning.to_string(), "reasoning");
        assert_eq!(TaskType::StructuredOutput.to_string(), "structured_output");
        assert_eq!(TaskType::CodeGeneration.to_string(), "code_generation");
        assert_eq!(TaskType::CodeEdit.to_string(), "code_edit");
        assert_eq!(TaskType::General.to_string(), "general");
    }

    #[test]
    fn selection_source_display() {
        assert_eq!(SelectionSource::RlEngine.to_string(), "rl-engine");
        assert_eq!(SelectionSource::Default.to_string(), "default");
        assert_eq!(SelectionSource::UserOverride.to_string(), "user-override");
        assert_eq!(SelectionSource::ProviderFiltered.to_string(), "provider-filtered");
    }
}

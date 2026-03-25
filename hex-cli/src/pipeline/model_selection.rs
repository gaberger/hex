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

/// Default: cheap, fast models for each task type (~$0.008/app).
/// openrouter/auto is NOT used as default — it picks expensive frontier models
/// like Gemini 3 Pro ($0.05/step). Specific cheap models are better for cost control.
/// Public accessor for the general-purpose default model (used by supervisor fallback).
pub fn default_model_for_general() -> &'static str {
    default_model_for(TaskType::General)
}

fn default_model_for(task_type: TaskType) -> &'static str {
    match task_type {
        // openai/gpt-4o-mini — reliable free model that supports OpenRouter privacy settings.
        // Using a specific model ID is more reliable than "openrouter/free" dynamic routing.
        TaskType::Reasoning => "openai/gpt-4o-mini",
        TaskType::StructuredOutput => "openai/gpt-4o-mini",
        TaskType::CodeGeneration => "openai/gpt-4o-mini",
        TaskType::CodeEdit => "openai/gpt-4o-mini",
        TaskType::General => "openai/gpt-4o-mini",
    }
}

/// Provider-specific model mapping for `--provider` flag.
///
/// Returns the best model for a given provider + task type combination.
/// Returns `None` if the provider is unknown (caller falls back to default).
fn provider_model_for(provider: &str, task_type: TaskType) -> Option<&'static str> {
    match provider {
        // For openrouter, the defaults are already OpenRouter model IDs — use them directly.
        "openrouter" => Some(default_model_for(task_type)),
        "anthropic" => Some(match task_type {
            TaskType::Reasoning => "claude-sonnet-4-6",
            TaskType::StructuredOutput => "claude-sonnet-4-6",
            TaskType::CodeGeneration => "claude-sonnet-4-6",
            TaskType::CodeEdit => "claude-haiku-4-5-20251001",
            TaskType::General => "claude-haiku-4-5-20251001",
        }),
        "ollama" => Some(match task_type {
            TaskType::Reasoning => "qwen3.5:27b",
            TaskType::StructuredOutput => "qwen3.5:27b",
            TaskType::CodeGeneration => "qwen3.5:27b",
            TaskType::CodeEdit => "qwen3.5:9b",
            TaskType::General => "qwen3.5:9b",
        }),
        _ => None,
    }
}

/// Free-tier fallback: specific reliable free model.
/// Used when paid credits are exhausted (402/insufficient credits).
pub fn free_fallback_for(_task_type: TaskType) -> &'static str {
    "openai/gpt-4o-mini"
}

/// Ordered fallback chain: primary → alternatives (all privacy-compatible free models).
pub fn fallback_chain_for(task_type: TaskType) -> Vec<&'static str> {
    vec![
        default_model_for(task_type),                   // Primary: llama-3.1-8b
        "google/gemma-2-9b-it:free",                    // Fallback 1: Gemma 2
        "qwen/qwen-2.5-7b-instruct:free",               // Fallback 2: Qwen 2.5
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
    /// Selected from agent YAML definition (ADR-2603240130).
    YamlDefinition,
    /// Upgraded from YAML preferred to upgrade_to model after iteration threshold.
    YamlUpgrade,
}

impl std::fmt::Display for SelectionSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::RlEngine => write!(f, "rl-engine"),
            Self::Default => write!(f, "default"),
            Self::UserOverride => write!(f, "user-override"),
            Self::ProviderFiltered => write!(f, "provider-filtered"),
            Self::YamlDefinition => write!(f, "yaml-definition"),
            Self::YamlUpgrade => write!(f, "yaml-upgrade"),
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

    /// Select a model from a YAML agent definition (ADR-2603240130).
    ///
    /// Fallback chain: user override → YAML preferred → YAML fallback → swarm model_defaults → openrouter/free.
    /// Upgrade: if `iteration >= upgrade_after`, switch to the YAML `upgrade_to` model.
    ///
    /// # Arguments
    /// * `model_config` — the agent's `model:` section from YAML
    /// * `model_override` — if `Some`, skip YAML entirely and use this model
    /// * `iteration` — current feedback loop iteration (for upgrade logic)
    /// * `upgrade_after` — iteration threshold from YAML (default: 3)
    /// * `swarm_default` — model from swarm YAML `model_defaults` for this task type
    pub fn select_from_yaml(
        &self,
        model_config: &crate::pipeline::agent_def::ModelConfig,
        model_override: Option<&str>,
        iteration: u32,
        upgrade_after: u32,
        swarm_default: Option<&str>,
    ) -> SelectedModel {
        // Fast path: user override
        if let Some(model) = model_override {
            return SelectedModel {
                model_id: model.to_string(),
                state_key: None,
                action: None,
                source: SelectionSource::UserOverride,
            };
        }

        // Check upgrade condition
        if iteration >= upgrade_after {
            if let Some(upgrade_id) = model_config.upgrade_model_id() {
                debug!(
                    upgrade_to = upgrade_id,
                    iteration,
                    upgrade_after,
                    "YAML upgrade triggered"
                );
                return SelectedModel {
                    model_id: upgrade_id.to_string(),
                    state_key: None,
                    action: None,
                    source: SelectionSource::YamlUpgrade,
                };
            }
        }

        // Preferred model from YAML
        let preferred_id = model_config.preferred_model_id();
        if preferred_id != "openrouter/free" {
            return SelectedModel {
                model_id: preferred_id.to_string(),
                state_key: None,
                action: None,
                source: SelectionSource::YamlDefinition,
            };
        }

        // Fallback from YAML
        let fallback_id = model_config.fallback_model_id();
        if fallback_id != "openrouter/free" {
            return SelectedModel {
                model_id: fallback_id.to_string(),
                state_key: None,
                action: None,
                source: SelectionSource::YamlDefinition,
            };
        }

        // Swarm-level default
        if let Some(swarm_model) = swarm_default {
            return SelectedModel {
                model_id: swarm_model.to_string(),
                state_key: None,
                action: None,
                source: SelectionSource::YamlDefinition,
            };
        }

        // Ultimate fallback: use specific reliable free model
        SelectedModel {
            model_id: default_model_for(TaskType::General).to_string(),
            state_key: None,
            action: None,
            source: SelectionSource::Default,
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

        // Provider preference: select the best model for this provider + task type.
        if let Some(provider) = provider_preference {
            let model_id = if let Some(m) = provider_model_for(provider, task_type) {
                m.to_string()
            } else {
                warn!(
                    %provider, %task_type,
                    "unknown provider preference — falling back to default model"
                );
                default_model_for(task_type).to_string()
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
        // Reward = accuracy (primary) + speed bonus (secondary) - cost penalty.
        // Accuracy matters most: success/failure is the main signal.
        // Speed bonus uses realistic LLM latency thresholds (not 5s which no LLM hits).
        // Cost penalty is light — cheap fast models should win over cheap slow ones.
        let base = if success { 1.0 } else { -0.5 };
        let cost_penalty = -cost_usd * 5.0; // lighter penalty — speed matters more than cost
        let speed_bonus = if duration_ms < 10_000 {
            0.3 // very fast (<10s): strong bonus
        } else if duration_ms < 20_000 {
            0.2 // fast (<20s): good bonus
        } else if duration_ms < 45_000 {
            0.1 // acceptable (<45s): small bonus
        } else {
            -0.2 // slow (>45s): penalty — pipeline throughput suffers
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
            // Preserve the native provider/model format expected by inference.rs.
            // "model:openrouter:meta-llama/llama-4-maverick" -> "meta-llama/llama-4-maverick"
            let or_id = segment.trim_start_matches("model:openrouter:");
            return or_id.to_string();
        }
        if segment.starts_with("model:") {
            // Non-OpenRouter model from RL — map known variants.
            return match segment {
                "model:opus" => "anthropic/claude-opus-4-6".to_string(),
                "model:sonnet" => "anthropic/claude-sonnet-4-6".to_string(),
                "model:haiku" => "openai/gpt-4o-mini".to_string(),
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
    fn default_models_are_known() {
        for task_type in [
            TaskType::Reasoning,
            TaskType::StructuredOutput,
            TaskType::CodeGeneration,
            TaskType::CodeEdit,
            TaskType::General,
        ] {
            let model = default_model_for(task_type);
            assert!(
                model.contains('/'),
                "default for {:?} should be a provider/model ID, got: {}",
                task_type,
                model
            );
        }
    }

    #[test]
    fn extract_openrouter_model() {
        let action = "model:openrouter:meta-llama/llama-4-maverick|context:balanced";
        let model = extract_model_from_action(action, TaskType::CodeGeneration);
        // Native provider/model format preserved for inference routing
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
        assert_eq!(model, default_model_for(TaskType::Reasoning));
    }

    #[test]
    fn extract_model_unknown_variant_falls_back() {
        let action = "model:unknown_thing|context:balanced";
        let model = extract_model_from_action(action, TaskType::CodeEdit);
        assert_eq!(model, default_model_for(TaskType::CodeEdit));
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
        assert_eq!(SelectionSource::YamlDefinition.to_string(), "yaml-definition");
        assert_eq!(SelectionSource::YamlUpgrade.to_string(), "yaml-upgrade");
    }

    #[test]
    fn yaml_select_preferred() {
        use crate::pipeline::agent_def::ModelConfig;
        let config = ModelConfig {
            tier: 2,
            preferred: Some("sonnet".into()),
            fallback: Some("haiku".into()),
            upgrade_to: Some("opus".into()),
            upgrade_condition: None,
            reasoning: None,
        };
        let selector = ModelSelector::from_env();
        let selected = selector.select_from_yaml(&config, None, 1, 3, None);
        assert_eq!(selected.model_id, "claude-sonnet-4-6");
        assert_eq!(selected.source, SelectionSource::YamlDefinition);
    }

    #[test]
    fn yaml_select_upgrade_after_threshold() {
        use crate::pipeline::agent_def::ModelConfig;
        let config = ModelConfig {
            tier: 2,
            preferred: Some("sonnet".into()),
            fallback: Some("haiku".into()),
            upgrade_to: Some("opus".into()),
            upgrade_condition: None,
            reasoning: None,
        };
        let selector = ModelSelector::from_env();
        // iteration=3, upgrade_after=3 → triggers upgrade
        let selected = selector.select_from_yaml(&config, None, 3, 3, None);
        assert_eq!(selected.model_id, "claude-opus-4-6");
        assert_eq!(selected.source, SelectionSource::YamlUpgrade);
    }

    #[test]
    fn yaml_select_user_override_wins() {
        use crate::pipeline::agent_def::ModelConfig;
        let config = ModelConfig {
            tier: 2,
            preferred: Some("sonnet".into()),
            fallback: Some("haiku".into()),
            upgrade_to: Some("opus".into()),
            upgrade_condition: None,
            reasoning: None,
        };
        let selector = ModelSelector::from_env();
        let selected = selector.select_from_yaml(&config, Some("my-custom-model"), 5, 3, None);
        assert_eq!(selected.model_id, "my-custom-model");
        assert_eq!(selected.source, SelectionSource::UserOverride);
    }

    #[test]
    fn yaml_select_fallback_chain() {
        use crate::pipeline::agent_def::ModelConfig;
        // No preferred, no fallback → swarm default
        let config = ModelConfig::default();
        let selector = ModelSelector::from_env();
        let selected = selector.select_from_yaml(&config, None, 1, 3, Some("deepseek/deepseek-r1"));
        assert_eq!(selected.model_id, "deepseek/deepseek-r1");
        assert_eq!(selected.source, SelectionSource::YamlDefinition);

        // No preferred, no fallback, no swarm default → openai/gpt-4o-mini
        let selected = selector.select_from_yaml(&config, None, 1, 3, None);
        assert_eq!(selected.model_id, "openai/gpt-4o-mini");
        assert_eq!(selected.source, SelectionSource::Default);
    }
}

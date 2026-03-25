//! Declarative agent & swarm definitions parsed from YAML (ADR-2603240130).
//!
//! Agent YAMLs live in `hex-cli/assets/agents/hex/hex/` and are embedded via
//! `rust-embed` at compile time. Two workflow variants exist:
//!   - **Coder-style**: `workflow.phases[]` + `feedback_loop` (TDD loop)
//!   - **Planner-style**: `workflow.steps[]` + `escalation` (decomposition)
//!
//! Swarm YAMLs live in `hex-cli/assets/swarms/` and define agent composition,
//! objectives, iteration limits, and grade thresholds.

use std::collections::HashMap;

use serde::de::{self, Deserializer};
use serde::Deserialize;
use tracing::warn;

use crate::assets::Assets;

// ── Custom Deserializers ────────────────────────────────────────────────

/// Deserialize `constraints` which can be:
/// - Vec<String> (hex-coder, planner)
/// - Vec<Map> with structured constraint entries (adr-reviewer)
/// - Map<String, Value> (rust-refactorer)
/// All are normalized to Vec<String> by stringifying non-string entries.
fn deserialize_constraints<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: Deserializer<'de>,
{
    let value = serde_yaml::Value::deserialize(deserializer)?;
    match value {
        serde_yaml::Value::Sequence(seq) => {
            Ok(seq.into_iter().map(|v| match v {
                serde_yaml::Value::String(s) => s,
                other => serde_yaml::to_string(&other).unwrap_or_default(),
            }).collect())
        }
        serde_yaml::Value::Mapping(map) => {
            // Convert map entries to "key: value" strings
            Ok(map.into_iter().map(|(k, v)| {
                let key = match k {
                    serde_yaml::Value::String(s) => s,
                    other => serde_yaml::to_string(&other).unwrap_or_default(),
                };
                let val = match v {
                    serde_yaml::Value::String(s) => s,
                    other => serde_yaml::to_string(&other).unwrap_or_default(),
                };
                format!("{}: {}", key, val)
            }).collect())
        }
        serde_yaml::Value::Null => Ok(Vec::new()),
        _ => Err(de::Error::custom("expected sequence or mapping for constraints")),
    }
}

/// Deserialize `tools` which can be:
/// - `{required: [...], optional: [...], inbox: [...]}` (hex-coder)
/// - `[string, string, ...]` (scaffold-validator)
fn deserialize_tools<'de, D>(deserializer: D) -> Result<Option<ToolsConfig>, D::Error>
where
    D: Deserializer<'de>,
{
    let value = serde_yaml::Value::deserialize(deserializer)?;
    match value {
        serde_yaml::Value::Null => Ok(None),
        serde_yaml::Value::Mapping(_) => {
            let tc: ToolsConfig = serde_yaml::from_value(value)
                .map_err(de::Error::custom)?;
            Ok(Some(tc))
        }
        serde_yaml::Value::Sequence(seq) => {
            let tools: Vec<String> = seq.into_iter().filter_map(|v| match v {
                serde_yaml::Value::String(s) => Some(s),
                _ => None,
            }).collect();
            Ok(Some(ToolsConfig {
                required: tools,
                optional: Vec::new(),
                inbox: Vec::new(),
            }))
        }
        _ => Err(de::Error::custom("expected mapping or sequence for tools")),
    }
}

/// Deserialize workflow phase steps which can be:
/// - `["string", "string"]` (hex-coder)
/// - `[{id: ..., action: ...}, ...]` (feature-developer, swarm-coordinator)
fn deserialize_step_items<'de, D>(deserializer: D) -> Result<Vec<serde_yaml::Value>, D::Error>
where
    D: Deserializer<'de>,
{
    let value = serde_yaml::Value::deserialize(deserializer)?;
    match value {
        serde_yaml::Value::Sequence(seq) => Ok(seq),
        serde_yaml::Value::Null => Ok(Vec::new()),
        _ => Err(de::Error::custom("expected sequence for steps")),
    }
}

// ── Agent Definition ────────────────────────────────────────────────────

/// Top-level agent definition parsed from a YAML file.
#[derive(Debug, Clone, Deserialize)]
pub struct AgentDefinition {
    pub name: String,
    #[serde(rename = "type", default)]
    pub agent_type: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub model: ModelConfig,
    #[serde(default)]
    pub context: Option<ContextConfig>,
    /// Constraints: can be Vec<String>, Vec<Map>, or a Map in different agent YAMLs.
    #[serde(default, deserialize_with = "deserialize_constraints")]
    pub constraints: Vec<String>,
    /// Tools: can be `{required, optional, inbox}` or a flat `Vec<String>`.
    #[serde(default, deserialize_with = "deserialize_tools")]
    pub tools: Option<ToolsConfig>,
    #[serde(default)]
    pub inputs: HashMap<String, InputSpec>,
    #[serde(default)]
    pub outputs: HashMap<String, OutputSpec>,
    #[serde(default)]
    pub workflow: Option<WorkflowConfig>,
    #[serde(default)]
    pub escalation: Option<EscalationConfig>,
    #[serde(default)]
    pub quality_thresholds: Option<QualityThresholds>,
}

// ── Model Config ────────────────────────────────────────────────────────

/// Model selection parameters — tier, preferred/fallback/upgrade.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ModelConfig {
    /// Model tier (1=cheap, 2=mid, 3=expensive).
    #[serde(default)]
    pub tier: u8,
    /// Preferred model name (e.g. "sonnet", "opus", "haiku").
    #[serde(default)]
    pub preferred: Option<String>,
    /// Fallback model if preferred is unavailable.
    #[serde(default)]
    pub fallback: Option<String>,
    /// Model to upgrade to under escalation conditions.
    #[serde(default)]
    pub upgrade_to: Option<String>,
    /// Human-readable condition for upgrade (parsed by supervisor).
    #[serde(default)]
    pub upgrade_condition: Option<String>,
    /// Reasoning for model choice (documentation only).
    #[serde(default)]
    pub reasoning: Option<String>,
}

impl ModelConfig {
    /// Map a YAML model name (sonnet/haiku/opus) to an OpenRouter-compatible model ID.
    ///
    /// Uses OpenRouter vendor-namespaced IDs (anthropic/...) so they route correctly
    /// regardless of whether the backend is Anthropic direct or OpenRouter.
    pub fn resolve_model_id(_name: &str) -> &'static str {
        "openai/gpt-4o-mini"
    }

    /// Preferred model ID, resolved from YAML name.
    pub fn preferred_model_id(&self) -> &'static str {
        self.preferred
            .as_deref()
            .map(Self::resolve_model_id)
            .unwrap_or("openai/gpt-4o-mini")
    }

    /// Fallback model ID, resolved from YAML name.
    pub fn fallback_model_id(&self) -> &'static str {
        self.fallback
            .as_deref()
            .map(Self::resolve_model_id)
            .unwrap_or("openai/gpt-4o-mini")
    }

    /// Upgrade model ID, resolved from YAML name.
    pub fn upgrade_model_id(&self) -> Option<&'static str> {
        self.upgrade_to.as_deref().map(Self::resolve_model_id)
    }
}

// ── Context Config ──────────────────────────────────────────────────────

/// How an agent loads context — supports both `load_strategy` (coder)
/// and `load_on_start` + `load_on_demand` (planner) patterns.
#[derive(Debug, Clone, Deserialize)]
pub struct ContextConfig {
    /// Coder-style: explicit L1/L2/L3 load levels with glob scopes.
    #[serde(default)]
    pub load_strategy: Vec<LoadStrategyEntry>,
    /// Planner-style: files loaded at startup.
    #[serde(default)]
    pub load_on_start: Vec<LoadOnStartEntry>,
    /// Planner-style: files loaded on demand with trigger conditions.
    #[serde(default)]
    pub load_on_demand: Vec<LoadOnDemandEntry>,
    /// Token budget allocation.
    #[serde(default)]
    pub token_budget: Option<TokenBudget>,
}

/// A single entry in a coder-style load_strategy.
#[derive(Debug, Clone, Deserialize)]
pub struct LoadStrategyEntry {
    /// Context level: L0 (file list), L1 (AST summary), L2 (signatures), L3 (full source).
    pub level: String,
    /// Glob scope (e.g. "src/core/ports/**"). May contain `{{placeholders}}`.
    pub scope: String,
    /// Human-readable purpose.
    #[serde(default)]
    pub purpose: Option<String>,
    /// When to load: "startup" (default) or "on_demand".
    #[serde(default)]
    pub load: Option<String>,
}

/// Planner-style load_on_start entry.
#[derive(Debug, Clone, Deserialize)]
pub struct LoadOnStartEntry {
    pub path: String,
    #[serde(default)]
    pub level: Option<String>,
    #[serde(default)]
    pub purpose: Option<String>,
}

/// Planner-style load_on_demand entry.
#[derive(Debug, Clone, Deserialize)]
pub struct LoadOnDemandEntry {
    pub path: String,
    #[serde(default)]
    pub level: Option<String>,
    #[serde(default)]
    pub trigger: Option<String>,
}

/// Token budget — max tokens and per-category allocation.
#[derive(Debug, Clone, Deserialize)]
pub struct TokenBudget {
    #[serde(default)]
    pub max: u64,
    #[serde(default)]
    pub reserved_response: u64,
    /// Named allocations (e.g. "port_interfaces": 5000).
    #[serde(default)]
    pub allocation: HashMap<String, u64>,
}

// ── Tools Config ────────────────────────────────────────────────────────

/// Tools the agent requires and optionally uses.
#[derive(Debug, Clone, Deserialize)]
pub struct ToolsConfig {
    #[serde(default)]
    pub required: Vec<String>,
    #[serde(default)]
    pub optional: Vec<String>,
    #[serde(default)]
    pub inbox: Vec<String>,
}

// ── Input / Output Specs ────────────────────────────────────────────────

/// Input parameter spec for an agent.
#[derive(Debug, Clone, Deserialize)]
pub struct InputSpec {
    #[serde(rename = "type")]
    pub input_type: String,
    #[serde(default)]
    pub required: bool,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(rename = "enum", default)]
    pub enum_values: Vec<String>,
    /// Default value — can be a string, list, or other YAML value.
    #[serde(default)]
    pub default: Option<serde_yaml::Value>,
}

/// Output spec for an agent.
#[derive(Debug, Clone, Deserialize)]
pub struct OutputSpec {
    /// Output type — optional because some agent YAMLs omit it.
    #[serde(rename = "type", default)]
    pub output_type: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    /// Nested schema (free-form for flexibility).
    #[serde(default)]
    pub schema: Option<serde_yaml::Value>,
}

// ── Workflow Config ─────────────────────────────────────────────────────

/// Workflow definition — supports both coder (phases) and planner (steps) patterns.
#[derive(Debug, Clone, Deserialize)]
pub struct WorkflowConfig {
    /// Coder-style: ordered TDD phases with blocking gates.
    #[serde(default)]
    pub phases: Vec<WorkflowPhase>,
    /// Planner-style: sequential decomposition steps.
    #[serde(default)]
    pub steps: Vec<WorkflowStep>,
    /// Coder-style: compile/lint/test feedback loop.
    #[serde(default)]
    pub feedback_loop: Option<FeedbackLoopConfig>,
    /// Commit configuration.
    #[serde(default)]
    pub commit: Option<CommitConfig>,
}

impl WorkflowConfig {
    /// True if this is a phase-based workflow (coder-style TDD).
    pub fn is_phase_based(&self) -> bool {
        !self.phases.is_empty()
    }

    /// True if this is a step-based workflow (planner-style decomposition).
    pub fn is_step_based(&self) -> bool {
        !self.steps.is_empty()
    }
}

/// A single TDD phase (e.g. pre_validate → red → green → refactor).
#[derive(Debug, Clone, Deserialize)]
pub struct WorkflowPhase {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    /// Steps: can be plain strings or structured maps in different agent YAMLs.
    #[serde(default, deserialize_with = "deserialize_step_items")]
    pub steps: Vec<serde_yaml::Value>,
    #[serde(default)]
    pub gate: Option<PhaseGate>,
}

/// A gate that blocks phase advancement.
#[derive(Debug, Clone, Deserialize)]
pub struct PhaseGate {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub blocking: bool,
    #[serde(default)]
    pub on_fail: Option<String>,
    #[serde(default)]
    pub required_categories: Vec<String>,
    #[serde(default)]
    pub recommended_categories: Vec<String>,
}

/// A planner-style workflow step.
#[derive(Debug, Clone, Deserialize)]
pub struct WorkflowStep {
    pub id: String,
    pub description: String,
    #[serde(default)]
    pub action: Option<String>,
}

// ── Feedback Loop ───────────────────────────────────────────────────────

/// Compile/lint/test feedback loop with iteration limits.
#[derive(Debug, Clone, Deserialize)]
pub struct FeedbackLoopConfig {
    #[serde(default = "default_max_iterations")]
    pub max_iterations: u32,
    #[serde(default)]
    pub gates: Vec<FeedbackGate>,
    #[serde(default)]
    pub on_max_iterations: Option<OnMaxIterations>,
}

fn default_max_iterations() -> u32 {
    5
}

/// A single feedback gate (compile, lint, or test).
#[derive(Debug, Clone, Deserialize)]
pub struct FeedbackGate {
    pub name: String,
    /// Language-specific commands: { "typescript": "npx tsc --noEmit", "rust": "cargo check" }
    #[serde(default)]
    pub command: HashMap<String, String>,
    #[serde(default)]
    pub timeout_ms: u64,
    #[serde(default)]
    pub on_fail: Option<String>,
}

/// What to do when max iterations are exhausted.
#[derive(Debug, Clone, Deserialize)]
pub struct OnMaxIterations {
    #[serde(default)]
    pub action: String,
    #[serde(default)]
    pub message: Option<String>,
}

/// Commit configuration after all gates pass.
#[derive(Debug, Clone, Deserialize)]
pub struct CommitConfig {
    /// Condition: "all_gates_pass".
    #[serde(rename = "when", default)]
    pub commit_when: String,
    #[serde(default)]
    pub message_template: Option<String>,
    #[serde(default)]
    pub include: Vec<String>,
}

// ── Escalation (planner-style) ──────────────────────────────────────────

/// Escalation rules for planner agents.
#[derive(Debug, Clone, Deserialize)]
pub struct EscalationConfig {
    #[serde(default)]
    pub conditions: Vec<String>,
    #[serde(default)]
    pub action: Option<String>,
}

// ── Quality Thresholds ──────────────────────────────────────────────────

/// Per-agent quality gates.
#[derive(Debug, Clone, Deserialize)]
pub struct QualityThresholds {
    #[serde(default)]
    pub test_coverage: Option<u32>,
    #[serde(default)]
    pub max_lint_warnings: Option<u32>,
    #[serde(default)]
    pub max_file_lines: Option<u32>,
    #[serde(default)]
    pub max_function_lines: Option<u32>,
    #[serde(default)]
    pub max_cyclomatic_complexity: Option<u32>,
}

impl Default for QualityThresholds {
    fn default() -> Self {
        Self {
            test_coverage: Some(80),
            max_lint_warnings: Some(5),
            max_file_lines: Some(500),
            max_function_lines: Some(50),
            max_cyclomatic_complexity: Some(10),
        }
    }
}

// ── Swarm Composition ───────────────────────────────────────────────────

/// Top-level swarm definition (e.g. dev-pipeline.yml).
#[derive(Debug, Clone, Deserialize)]
pub struct SwarmComposition {
    pub name: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub topology: String,
    #[serde(default)]
    pub inference_defaults: Option<InferenceDefaults>,
    #[serde(default)]
    pub model_defaults: Option<HashMap<String, String>>,
    #[serde(default)]
    pub agents: Vec<SwarmAgentEntry>,
    #[serde(default)]
    pub objectives: Vec<SwarmObjective>,
    #[serde(default)]
    pub iteration: Option<IterationConfig>,
    #[serde(default)]
    pub grade_thresholds: Option<GradeThresholds>,
}

/// Inference defaults applied to all agents unless overridden.
#[derive(Debug, Clone, Deserialize)]
pub struct InferenceDefaults {
    #[serde(default)]
    pub provider: String,
    #[serde(default)]
    pub endpoint: String,
    #[serde(default)]
    pub fallback_chain: Vec<String>,
    #[serde(default)]
    pub temperature: f32,
    #[serde(default)]
    pub timeout_ms: u64,
    #[serde(default)]
    pub retry: Option<RetryConfig>,
}

/// Retry settings for inference calls.
#[derive(Debug, Clone, Deserialize)]
pub struct RetryConfig {
    #[serde(default)]
    pub max_attempts: u32,
    #[serde(default)]
    pub backoff_ms: u64,
    #[serde(default)]
    pub backoff_free_ms: u64,
}

/// A single agent entry in a swarm composition.
#[derive(Debug, Clone, Deserialize)]
pub struct SwarmAgentEntry {
    pub role: String,
    #[serde(default)]
    pub description: Option<String>,
    /// Reference to the agent YAML definition file.
    #[serde(default)]
    pub definition: Option<String>,
    #[serde(default)]
    pub prompt: Option<String>,
    #[serde(default)]
    pub cardinality: Option<String>,
    #[serde(default)]
    pub tier_scope: Option<String>,
    #[serde(default)]
    pub depends_on: Vec<String>,
    #[serde(default)]
    pub parallel_with: Vec<String>,
    /// When to run (e.g. "has_primary_adapters", "final_tier").
    #[serde(rename = "when", default)]
    pub run_when: Option<String>,
    #[serde(default)]
    pub inference: Option<SwarmAgentInference>,
    #[serde(default)]
    pub context: Option<SwarmAgentContext>,
    #[serde(default)]
    pub output: Option<SwarmAgentOutput>,
    #[serde(default)]
    pub execute: Option<SwarmAgentExecute>,
    #[serde(default)]
    pub post_execute: Vec<String>,
}

/// Per-agent inference override within a swarm.
#[derive(Debug, Clone, Deserialize)]
pub struct SwarmAgentInference {
    #[serde(default)]
    pub task_type: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub fallback: Option<String>,
    #[serde(default)]
    pub max_tokens: Option<u32>,
    #[serde(default)]
    pub temperature: Option<f32>,
    #[serde(default)]
    pub upgrade: Option<SwarmUpgradeConfig>,
}

/// Upgrade config within a swarm agent entry.
#[derive(Debug, Clone, Deserialize)]
pub struct SwarmUpgradeConfig {
    pub after_iterations: u32,
    pub to: String,
}

/// Per-agent context override within a swarm.
#[derive(Debug, Clone, Deserialize)]
pub struct SwarmAgentContext {
    #[serde(default)]
    pub strategy: Option<String>,
    #[serde(default)]
    pub token_budget: Option<u64>,
    #[serde(default)]
    pub includes: Vec<String>,
}

/// Per-agent output spec within a swarm.
#[derive(Debug, Clone, Deserialize)]
pub struct SwarmAgentOutput {
    #[serde(rename = "type", default)]
    pub output_type: String,
    #[serde(default)]
    pub schema: Option<String>,
    #[serde(default)]
    pub write_to: Option<String>,
    #[serde(default)]
    pub files: Vec<String>,
}

/// Non-LLM execution (e.g. hex-analyzer runs a CLI command).
#[derive(Debug, Clone, Deserialize)]
pub struct SwarmAgentExecute {
    pub command: String,
    #[serde(default)]
    pub parse: Option<String>,
    #[serde(default)]
    pub timeout_ms: Option<u64>,
}

/// Swarm-level objective (goal-driven loop).
#[derive(Debug, Clone, Deserialize)]
pub struct SwarmObjective {
    pub id: String,
    #[serde(default)]
    pub required: bool,
    #[serde(default)]
    pub agent: Option<String>,
    #[serde(default)]
    pub depends_on: Vec<String>,
    /// Evaluation: string or structured command.
    #[serde(default)]
    pub evaluate: Option<serde_yaml::Value>,
    #[serde(default)]
    pub threshold: Option<serde_yaml::Value>,
    #[serde(rename = "when", default)]
    pub run_when: Option<String>,
}

/// Iteration limits for the objective loop.
#[derive(Debug, Clone, Deserialize)]
pub struct IterationConfig {
    #[serde(default)]
    pub max_per_tier: u32,
    #[serde(default)]
    pub max_total: u32,
    #[serde(default)]
    pub on_max_iterations: String,
    #[serde(default)]
    pub cooldown_ms: u64,
}

/// Grade thresholds for different dev modes.
#[derive(Debug, Clone, Deserialize)]
pub struct GradeThresholds {
    #[serde(default)]
    pub interactive: Option<GradeEntry>,
    #[serde(default)]
    pub auto: Option<GradeEntry>,
    #[serde(default)]
    pub quick: Option<GradeEntry>,
}

/// A single grade threshold entry.
#[derive(Debug, Clone, Deserialize)]
pub struct GradeEntry {
    /// Minimum grade: "A", "B", "C", or null (no gate).
    pub minimum: Option<String>,
}

// ── Loaders ─────────────────────────────────────────────────────────────

const AGENT_YAML_PREFIX: &str = "agents/hex/hex/";
const SWARM_YAML_PREFIX: &str = "swarms/";

impl AgentDefinition {
    /// Load all agent definitions from embedded YAML assets.
    ///
    /// Returns a map from agent name → definition. Malformed YAMLs are
    /// logged as warnings and skipped.
    pub fn load_all() -> HashMap<String, AgentDefinition> {
        let mut defs = HashMap::new();
        for path in Assets::iter() {
            let path_str = path.as_ref();
            if !path_str.starts_with(AGENT_YAML_PREFIX) || !path_str.ends_with(".yml") {
                continue;
            }
            let Some(content) = Assets::get_str(path_str) else {
                warn!(path = path_str, "embedded agent YAML not readable");
                continue;
            };
            match serde_yaml::from_str::<AgentDefinition>(&content) {
                Ok(def) => {
                    defs.insert(def.name.clone(), def);
                }
                Err(e) => {
                    warn!(path = path_str, error = %e, "skipping malformed agent YAML");
                }
            }
        }
        defs
    }

    /// Load a single agent definition by name (e.g. "hex-coder").
    pub fn load(name: &str) -> Option<AgentDefinition> {
        let path = format!("{}{}.yml", AGENT_YAML_PREFIX, name);
        let content = Assets::get_str(&path)?;
        match serde_yaml::from_str::<AgentDefinition>(&content) {
            Ok(def) => Some(def),
            Err(e) => {
                warn!(name, error = %e, "failed to parse agent YAML");
                None
            }
        }
    }
}

impl SwarmComposition {
    /// Load all swarm compositions from embedded YAML assets.
    pub fn load_all() -> HashMap<String, SwarmComposition> {
        let mut comps = HashMap::new();
        for path in Assets::iter() {
            let path_str = path.as_ref();
            if !path_str.starts_with(SWARM_YAML_PREFIX) || !path_str.ends_with(".yml") {
                continue;
            }
            let Some(content) = Assets::get_str(path_str) else {
                warn!(path = path_str, "embedded swarm YAML not readable");
                continue;
            };
            match serde_yaml::from_str::<SwarmComposition>(&content) {
                Ok(comp) => {
                    comps.insert(comp.name.clone(), comp);
                }
                Err(e) => {
                    warn!(path = path_str, error = %e, "skipping malformed swarm YAML");
                }
            }
        }
        comps
    }

    /// Load a single swarm composition by name (e.g. "dev-pipeline").
    pub fn load(name: &str) -> Option<SwarmComposition> {
        let path = format!("{}{}.yml", SWARM_YAML_PREFIX, name);
        let content = Assets::get_str(&path)?;
        match serde_yaml::from_str::<SwarmComposition>(&content) {
            Ok(comp) => Some(comp),
            Err(e) => {
                warn!(name, error = %e, "failed to parse swarm YAML");
                None
            }
        }
    }
}

// ── Tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_hex_coder_yaml() {
        let def = AgentDefinition::load("hex-coder")
            .expect("hex-coder.yml should parse");

        assert_eq!(def.name, "hex-coder");
        assert_eq!(def.agent_type, "coder");
        assert_eq!(def.model.tier, 2);
        assert_eq!(def.model.preferred.as_deref(), Some("sonnet"));
        assert_eq!(def.model.fallback.as_deref(), Some("haiku"));
        assert_eq!(def.model.upgrade_to.as_deref(), Some("opus"));

        // Model ID resolution
        assert_eq!(def.model.preferred_model_id(), "claude-sonnet-4-6");
        assert_eq!(def.model.fallback_model_id(), "claude-haiku-4-5-20251001");
        assert_eq!(def.model.upgrade_model_id(), Some("claude-opus-4-6"));

        // Context load_strategy
        let ctx = def.context.expect("hex-coder should have context");
        assert!(ctx.load_strategy.len() >= 4, "should have L1/L2/L3 entries");
        assert_eq!(ctx.load_strategy[0].level, "L1");

        // Token budget
        let budget = ctx.token_budget.expect("should have token budget");
        assert_eq!(budget.max, 100000);
        assert_eq!(budget.reserved_response, 20000);
        assert!(budget.allocation.contains_key("port_interfaces"));

        // Workflow phases (coder-style)
        let wf = def.workflow.expect("hex-coder should have workflow");
        assert!(wf.is_phase_based());
        assert!(!wf.is_step_based());
        assert_eq!(wf.phases.len(), 5);
        assert_eq!(wf.phases[0].id, "pre_validate");
        assert_eq!(wf.phases[1].id, "red");
        assert_eq!(wf.phases[2].id, "green");
        assert_eq!(wf.phases[3].id, "refactor");
        assert_eq!(wf.phases[4].id, "test_coverage_gate");

        // Phase gates
        let gate = wf.phases[0].gate.as_ref().expect("pre_validate should have gate");
        assert!(gate.blocking);
        assert_eq!(gate.name, "boundary_check");

        // Feedback loop
        let fl = wf.feedback_loop.expect("hex-coder should have feedback loop");
        assert_eq!(fl.max_iterations, 5);
        assert_eq!(fl.gates.len(), 3);
        assert_eq!(fl.gates[0].name, "compile");
        assert_eq!(fl.gates[1].name, "lint");
        assert_eq!(fl.gates[2].name, "test");

        // Language-specific commands
        assert!(fl.gates[0].command.contains_key("typescript"));
        assert!(fl.gates[0].command.contains_key("rust"));

        // Quality thresholds
        let qt = def.quality_thresholds.expect("hex-coder should have thresholds");
        assert_eq!(qt.test_coverage, Some(80));
        assert_eq!(qt.max_lint_warnings, Some(5));
        assert_eq!(qt.max_file_lines, Some(500));
    }

    #[test]
    fn parse_planner_yaml() {
        let def = AgentDefinition::load("planner")
            .expect("planner.yml should parse");

        assert_eq!(def.name, "planner");
        assert_eq!(def.agent_type, "planner");
        assert_eq!(def.model.tier, 3);
        assert_eq!(def.model.preferred.as_deref(), Some("opus"));
        assert_eq!(def.model.fallback.as_deref(), Some("sonnet"));

        // Context: planner uses load_on_start/load_on_demand, not load_strategy
        let ctx = def.context.expect("planner should have context");
        assert!(ctx.load_on_start.len() >= 3);
        assert!(!ctx.load_on_demand.is_empty());

        // Workflow steps (planner-style)
        let wf = def.workflow.expect("planner should have workflow");
        assert!(wf.is_step_based());
        assert!(!wf.is_phase_based());
        assert!(wf.steps.len() >= 4);
        assert_eq!(wf.steps[0].id, "analyze-requirements");

        // Escalation
        let esc = def.escalation.expect("planner should have escalation");
        assert!(!esc.conditions.is_empty());
    }

    #[test]
    fn load_all_agents_parses_all_14() {
        let defs = AgentDefinition::load_all();
        // We have 14 agent YAMLs
        assert!(
            defs.len() >= 10,
            "should parse most agent YAMLs, got {}",
            defs.len()
        );
        assert!(defs.contains_key("hex-coder"));
        assert!(defs.contains_key("planner"));
    }

    #[test]
    fn parse_dev_pipeline_swarm() {
        let comp = SwarmComposition::load("dev-pipeline")
            .expect("dev-pipeline.yml should parse");

        assert_eq!(comp.name, "dev-pipeline");
        assert_eq!(comp.topology, "hex-pipeline");

        // Inference defaults
        let inf = comp.inference_defaults.expect("should have inference defaults");
        assert_eq!(inf.provider, "openrouter");

        // Model defaults
        let md = comp.model_defaults.expect("should have model defaults");
        assert!(md.contains_key("reasoning"));
        assert!(md.contains_key("code_generation"));

        // Agents
        assert!(comp.agents.len() >= 6, "should have 6+ agent entries");
        let coder = comp.agents.iter().find(|a| a.role == "hex-coder")
            .expect("should have hex-coder entry");
        assert_eq!(coder.cardinality.as_deref(), Some("per_workplan_step"));

        // Objectives
        assert!(comp.objectives.len() >= 5);
        let compile = comp.objectives.iter().find(|o| o.id == "CodeCompiles")
            .expect("should have CodeCompiles objective");
        assert!(compile.required);

        // Iteration
        let iter = comp.iteration.expect("should have iteration config");
        assert_eq!(iter.max_per_tier, 5);
        assert_eq!(iter.max_total, 25);

        // Grade thresholds
        let grades = comp.grade_thresholds.expect("should have grade thresholds");
        assert!(grades.interactive.is_some());
    }

    #[test]
    fn load_all_swarms() {
        let comps = SwarmComposition::load_all();
        assert!(
            comps.len() >= 5,
            "should parse most swarm YAMLs, got {}",
            comps.len()
        );
        assert!(comps.contains_key("dev-pipeline"));
    }

    #[test]
    fn model_id_resolution() {
        assert_eq!(ModelConfig::resolve_model_id("opus"), "claude-opus-4-6");
        assert_eq!(ModelConfig::resolve_model_id("sonnet"), "claude-sonnet-4-6");
        assert_eq!(ModelConfig::resolve_model_id("haiku"), "claude-haiku-4-5-20251001");
        assert_eq!(ModelConfig::resolve_model_id("unknown"), "openrouter/free");
    }
}

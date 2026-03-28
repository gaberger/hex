//! Swarm composition configuration loaded from YAML (ADR-2603241230, step 8).
//!
//! Replaces the hardcoded `roles_for_workplan()` in supervisor.rs with a
//! data-driven approach: the dev-pipeline.yml defines which agents participate,
//! their dependencies, cardinality, and conditional activation (`when` guards).

use serde::Deserialize;

/// Top-level swarm definition, parsed from `assets/swarms/<name>.yml`.
#[derive(Debug, Deserialize)]
pub struct SwarmConfig {
    pub name: String,
    #[serde(default)]
    pub version: Option<String>,
    pub topology: String,
    #[serde(default)]
    pub description: String,

    /// Inference defaults applied to all agents unless overridden.
    #[serde(default)]
    pub inference_defaults: Option<serde_yaml::Value>,

    /// Per-task-type model defaults.
    #[serde(default)]
    pub model_defaults: Option<serde_yaml::Value>,

    /// Agent role definitions.
    pub agents: Vec<SwarmAgentConfig>,

    /// Objective definitions for the goal-driven loop.
    #[serde(default)]
    pub objectives: Vec<serde_yaml::Value>,

    /// Iteration control.
    #[serde(default)]
    pub iteration: Option<IterationConfig>,

    /// Grade thresholds per dev-mode.
    #[serde(default)]
    pub grade_thresholds: Option<serde_yaml::Value>,
}

/// A single agent role within the swarm.
#[derive(Debug, Deserialize)]
pub struct SwarmAgentConfig {
    pub role: String,
    #[serde(default)]
    pub description: Option<String>,
    /// Agent definition file reference (e.g. `hex-coder.yml`).
    #[serde(default)]
    pub definition: Option<String>,
    /// System prompt template (e.g. `agent-coder.md`).
    #[serde(default)]
    pub prompt: Option<String>,

    /// How many instances: `per_workplan_step`, `per_source_file`, `per_tier`,
    /// `per_issue`, `per_swarm`.
    #[serde(default)]
    pub cardinality: Option<String>,
    /// Restrict to `current` tier or `all`.
    #[serde(default)]
    pub tier_scope: Option<String>,

    /// Roles this agent depends on (must complete first).
    #[serde(default)]
    pub depends_on: Vec<String>,
    /// Roles this agent can run in parallel with.
    #[serde(default)]
    pub parallel_with: Vec<String>,
    /// Roles that trigger this agent (e.g. fixer triggered by reviewer).
    #[serde(default)]
    pub triggered_by: Vec<String>,

    /// Conditional activation guard:
    /// `has_primary_adapters`, `final_tier`, `objectives_unmet`, etc.
    #[serde(default)]
    pub when: Option<String>,

    /// Inference configuration (model, temperature, etc.).
    #[serde(default)]
    pub inference: Option<serde_yaml::Value>,
    /// Non-LLM command execution (e.g. `hex analyze`).
    #[serde(default)]
    pub execute: Option<serde_yaml::Value>,
    /// Context loading strategy.
    #[serde(default)]
    pub context: Option<serde_yaml::Value>,
    /// Output configuration.
    #[serde(default)]
    pub output: Option<serde_yaml::Value>,
    /// Post-execution steps.
    #[serde(default)]
    pub post_execute: Option<Vec<String>>,
}

/// Iteration limits for the objective loop.
#[derive(Debug, Deserialize)]
pub struct IterationConfig {
    #[serde(default = "default_max_per_tier")]
    pub max_per_tier: u32,
    #[serde(default = "default_max_total")]
    pub max_total: u32,
    #[serde(default)]
    pub on_max_iterations: Option<String>,
    #[serde(default)]
    pub cooldown_ms: Option<u64>,
}

fn default_max_per_tier() -> u32 {
    5
}
fn default_max_total() -> u32 {
    25
}

impl SwarmConfig {
    /// Load the default dev-pipeline from the embedded YAML asset.
    pub fn load_default() -> Self {
        let yaml = include_str!("../../assets/swarms/dev-pipeline.yml");
        serde_yaml::from_str(yaml).expect("failed to parse embedded dev-pipeline.yml")
    }

    /// Load a swarm config from an arbitrary file path.
    pub fn load_from(path: &str) -> anyhow::Result<Self> {
        let content = std::fs::read_to_string(path)?;
        Ok(serde_yaml::from_str(&content)?)
    }

    /// Maximum iterations per tier (from the `iteration` block, default 5).
    pub fn max_iterations_per_tier(&self) -> u32 {
        self.iteration
            .as_ref()
            .map(|i| i.max_per_tier)
            .unwrap_or(5)
    }

    /// Determine which agent roles should be active given the current context.
    ///
    /// Evaluates the `when` guard on each agent definition:
    /// - `has_primary_adapters` — include only if the workplan touches primary adapters
    /// - `final_tier` — include only on the last tier
    /// - `objectives_unmet` — include only when the objective loop has failures
    /// - `None` — always include (unconditional agent)
    ///
    /// Agents with `triggered_by` are included unconditionally here because
    /// whether they actually run is decided at dispatch time by the supervisor.
    pub fn roles_for_context(
        &self,
        has_primary_adapters: bool,
        is_final_tier: bool,
        _objectives_unmet: bool,
    ) -> Vec<String> {
        self.agents
            .iter()
            .filter(|a| match a.when.as_deref() {
                Some("has_primary_adapters") | Some("has_ui_adapters") => has_primary_adapters,
                Some("final_tier") => is_final_tier,
                // Agents triggered by upstream failures are always included in the
                // role set — the supervisor decides whether to actually dispatch them.
                Some("objectives_unmet") => true,
                Some(_) => true, // unknown condition — include conservatively
                None => true,    // unconditional
            })
            .map(|a| a.role.clone())
            .collect()
    }

    /// Return the dependency list for a given role.
    pub fn depends_on(&self, role: &str) -> Vec<String> {
        self.agents
            .iter()
            .find(|a| a.role == role)
            .map(|a| a.depends_on.clone())
            .unwrap_or_default()
    }

    /// Return the parallel_with list for a given role.
    pub fn parallel_with(&self, role: &str) -> Vec<String> {
        self.agents
            .iter()
            .find(|a| a.role == role)
            .map(|a| a.parallel_with.clone())
            .unwrap_or_default()
    }

    /// Return the [`AgentCardinality`] for a given role as defined in the swarm YAML.
    ///
    /// Falls back to [`AgentCardinality::PerWorkplanStep`] when the role is not
    /// found or has no explicit cardinality.
    pub fn cardinality_for_role(&self, role: &str) -> AgentCardinality {
        self.agents
            .iter()
            .find(|a| a.role == role)
            .and_then(|a| a.cardinality.as_deref())
            .map(|s| s.parse::<AgentCardinality>().unwrap_or(AgentCardinality::PerWorkplanStep))
            .unwrap_or(AgentCardinality::PerWorkplanStep)
    }
}

// ── Agent Cardinality ────────────────────────────────────────────────────

/// How many agent instances the supervisor spawns for a given role, as read
/// from the `cardinality` field in the swarm YAML (e.g. dev-pipeline.yml).
#[derive(Debug, Clone, PartialEq)]
pub enum AgentCardinality {
    /// One agent instance per workplan step (default for hex-coder).
    PerWorkplanStep,
    /// One agent instance per source file (hex-reviewer, hex-tester).
    PerSourceFile,
    /// One agent instance per tier (hex-analyzer, hex-ux).
    PerTier,
    /// One agent instance for the whole swarm (hex-documenter).
    PerSwarm,
    /// One agent instance per issue reported by an upstream agent (hex-fixer).
    PerIssue,
}

impl std::str::FromStr for AgentCardinality {
    type Err = std::convert::Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s {
            "per_source_file" => Self::PerSourceFile,
            "per_tier" => Self::PerTier,
            "per_swarm" => Self::PerSwarm,
            "per_issue" => Self::PerIssue,
            _ => Self::PerWorkplanStep, // default (covers "per_workplan_step" + unknown)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_default_parses_embedded_yaml() {
        let config = SwarmConfig::load_default();
        assert_eq!(config.name, "dev-pipeline");
        assert_eq!(config.topology, "hex-pipeline");
        assert!(!config.agents.is_empty(), "agents list should not be empty");

        // Verify known roles exist
        let roles: Vec<&str> = config.agents.iter().map(|a| a.role.as_str()).collect();
        assert!(roles.contains(&"hex-coder"));
        assert!(roles.contains(&"hex-reviewer"));
        assert!(roles.contains(&"hex-tester"));
        assert!(roles.contains(&"hex-fixer"));
    }

    #[test]
    fn roles_for_context_filters_conditional_agents() {
        let config = SwarmConfig::load_default();

        // Without primary adapters or final tier
        let roles = config.roles_for_context(false, false, false);
        assert!(roles.contains(&"hex-coder".to_string()));
        assert!(
            !roles.contains(&"hex-ux".to_string()),
            "hex-ux should be excluded without primary adapters"
        );
        assert!(
            !roles.contains(&"hex-documenter".to_string()),
            "hex-documenter should be excluded when not final tier"
        );

        // With primary adapters and final tier
        let roles_full = config.roles_for_context(true, true, false);
        assert!(roles_full.contains(&"hex-ux".to_string()));
        assert!(roles_full.contains(&"hex-documenter".to_string()));
    }

    #[test]
    fn max_iterations_from_config() {
        let config = SwarmConfig::load_default();
        assert_eq!(config.max_iterations_per_tier(), 5);
    }

    #[test]
    fn cardinality_for_known_roles() {
        let config = SwarmConfig::load_default();

        assert_eq!(
            config.cardinality_for_role("hex-coder"),
            AgentCardinality::PerWorkplanStep,
            "hex-coder cardinality should be per_workplan_step"
        );
        assert_eq!(
            config.cardinality_for_role("hex-reviewer"),
            AgentCardinality::PerSourceFile,
            "hex-reviewer cardinality should be per_source_file"
        );
        assert_eq!(
            config.cardinality_for_role("hex-tester"),
            AgentCardinality::PerSourceFile,
            "hex-tester cardinality should be per_source_file"
        );
        assert_eq!(
            config.cardinality_for_role("hex-analyzer"),
            AgentCardinality::PerTier,
            "hex-analyzer cardinality should be per_tier"
        );
        assert_eq!(
            config.cardinality_for_role("hex-documenter"),
            AgentCardinality::PerSwarm,
            "hex-documenter cardinality should be per_swarm"
        );
        assert_eq!(
            config.cardinality_for_role("hex-fixer"),
            AgentCardinality::PerIssue,
            "hex-fixer cardinality should be per_issue"
        );
    }

    #[test]
    fn cardinality_unknown_role_defaults_to_per_workplan_step() {
        let config = SwarmConfig::load_default();
        assert_eq!(
            config.cardinality_for_role("unknown-role"),
            AgentCardinality::PerWorkplanStep
        );
    }

    #[test]
    fn cardinality_from_str_all_variants() {
        assert_eq!("per_workplan_step".parse::<AgentCardinality>().unwrap(), AgentCardinality::PerWorkplanStep);
        assert_eq!("per_source_file".parse::<AgentCardinality>().unwrap(), AgentCardinality::PerSourceFile);
        assert_eq!("per_tier".parse::<AgentCardinality>().unwrap(), AgentCardinality::PerTier);
        assert_eq!("per_swarm".parse::<AgentCardinality>().unwrap(), AgentCardinality::PerSwarm);
        assert_eq!("per_issue".parse::<AgentCardinality>().unwrap(), AgentCardinality::PerIssue);
        assert_eq!("garbage".parse::<AgentCardinality>().unwrap(), AgentCardinality::PerWorkplanStep);
    }
}

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// An agent definition loaded from YAML — defines a persona the LLM adopts.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentDefinition {
    /// Unique agent identifier (e.g., "hex-coder", "planner")
    pub name: String,
    /// Human-readable description
    pub description: String,
    /// System prompt additions specific to this agent's role
    pub role_prompt: String,
    /// Which tools this agent is allowed to use (empty = all)
    #[serde(default)]
    pub allowed_tools: Vec<String>,
    /// Constraints on agent behavior
    #[serde(default)]
    pub constraints: AgentConstraints,
    /// Model override (default: use global setting)
    #[serde(default)]
    pub model: Option<String>,
    /// Max turns before the agent must stop
    #[serde(default = "default_max_turns")]
    pub max_turns: u32,
    /// Arbitrary metadata
    #[serde(default)]
    pub metadata: HashMap<String, String>,
}

fn default_max_turns() -> u32 {
    50
}

/// Behavioral constraints for an agent.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AgentConstraints {
    /// Files/directories the agent may NOT touch
    #[serde(default)]
    pub forbidden_paths: Vec<String>,
    /// Maximum file size the agent may write (bytes)
    #[serde(default)]
    pub max_file_size: Option<u64>,
    /// Whether the agent may execute bash commands
    #[serde(default = "default_true")]
    pub allow_bash: bool,
    /// Whether the agent may write to files
    #[serde(default = "default_true")]
    pub allow_write: bool,
    /// Hex architecture layer this agent operates in (for boundary enforcement)
    #[serde(default)]
    pub hex_layer: Option<String>,
}

fn default_true() -> bool {
    true
}

/// Runtime metrics for an agent execution session.
#[derive(Debug, Clone, Default)]
pub struct AgentMetrics {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub tool_calls: u32,
    pub turns: u32,
    pub files_read: u32,
    pub files_written: u32,
    pub duration_ms: u64,
    pub errors: u32,
}

impl AgentMetrics {
    pub fn total_tokens(&self) -> u64 {
        self.input_tokens + self.output_tokens
    }
}

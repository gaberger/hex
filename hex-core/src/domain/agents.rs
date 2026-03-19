use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// An agent definition loaded from YAML — defines a persona the LLM adopts.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentDefinition {
    pub name: String,
    pub description: String,
    pub role_prompt: String,
    #[serde(default)]
    pub allowed_tools: Vec<String>,
    #[serde(default)]
    pub constraints: AgentConstraints,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default = "default_max_turns")]
    pub max_turns: u32,
    #[serde(default)]
    pub metadata: HashMap<String, String>,
}

fn default_max_turns() -> u32 {
    50
}

/// Behavioral constraints for an agent.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AgentConstraints {
    #[serde(default)]
    pub forbidden_paths: Vec<String>,
    #[serde(default)]
    pub max_file_size: Option<u64>,
    #[serde(default = "default_true")]
    pub allow_bash: bool,
    #[serde(default = "default_true")]
    pub allow_write: bool,
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

/// Agent status in the fleet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentStatus {
    Spawning,
    Running,
    Completed,
    Failed,
    Stale,
    Dead,
}

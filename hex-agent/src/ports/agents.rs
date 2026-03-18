use crate::domain::AgentDefinition;
use async_trait::async_trait;

/// Port for loading agent definitions from YAML files.
#[async_trait]
pub trait AgentLoaderPort: Send + Sync {
    /// Load all agent definitions from the given directories.
    async fn load(&self, dirs: &[&str]) -> Result<Vec<AgentDefinition>, AgentLoadError>;

    /// Load a single agent by name.
    async fn load_by_name(&self, dirs: &[&str], name: &str) -> Result<AgentDefinition, AgentLoadError>;
}

#[derive(Debug, thiserror::Error)]
pub enum AgentLoadError {
    #[error("Agent '{name}' not found in {dirs:?}")]
    NotFound { name: String, dirs: Vec<String> },
    #[error("Failed to read agent file {path}: {reason}")]
    ReadError { path: String, reason: String },
    #[error("Failed to parse agent YAML in {path}: {reason}")]
    ParseError { path: String, reason: String },
}

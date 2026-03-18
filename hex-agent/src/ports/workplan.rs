use crate::domain::Workplan;
use async_trait::async_trait;

/// Port for loading and persisting workplan state.
#[async_trait]
pub trait WorkplanPort: Send + Sync {
    /// Load a workplan from a JSON file.
    async fn load(&self, path: &str) -> Result<Workplan, WorkplanError>;

    /// Save workplan state (task statuses) back to disk.
    async fn save(&self, path: &str, workplan: &Workplan) -> Result<(), WorkplanError>;
}

#[derive(Debug, thiserror::Error)]
pub enum WorkplanError {
    #[error("Workplan not found: {0}")]
    NotFound(String),
    #[error("Failed to parse workplan: {0}")]
    ParseError(String),
    #[error("Failed to write workplan: {0}")]
    WriteError(String),
}

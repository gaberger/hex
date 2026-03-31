use crate::domain::context::ContextVariables;

/// Error type for live context enrichment failures.
#[derive(Debug, thiserror::Error)]
pub enum LiveContextError {
    #[error("nexus unavailable: {0}")]
    NexusUnavailable(String),
    #[error("enrichment failed: {0}")]
    EnrichmentFailed(String),
}

/// Port for enriching ContextVariables with live hex project state.
/// Implementations call hex-nexus REST endpoints to populate fields
/// like architecture_score, relevant_adrs, ast_summary, etc.
/// All enrichment is best-effort: missing data sets fields to None, never errors.
#[async_trait::async_trait]
pub trait ILiveContextPort: Send + Sync {
    /// Enrich `vars` in-place with live project state.
    /// `task` is used as a search query for ADRs and memory.
    /// `files` are the target files for AST summary.
    async fn enrich(
        &self,
        vars: &mut ContextVariables,
        task: &str,
        files: &[String],
    ) -> Result<(), LiveContextError>;
}

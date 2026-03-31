/// Variables injected into agent system prompts for context engineering.
/// Fields are populated by ILiveContextPort implementations; all are optional
/// because enrichment is best-effort (missing data = None, never an error).
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct ContextVariables {
    /// Hexagonal architecture health score (0–100).
    pub architecture_score: Option<f32>,
    /// ADR titles/IDs relevant to the current task.
    pub relevant_adrs: Option<Vec<String>>,
    /// Token-efficient AST summary for the target files.
    pub ast_summary: Option<String>,
    /// Key/value pairs from hex memory relevant to the task.
    pub memory_snippets: Option<Vec<(String, String)>>,
    /// Raw git diff context (recent changes).
    pub git_diff: Option<String>,
    /// Current workplan phase, if any.
    pub workplan_phase: Option<String>,
}

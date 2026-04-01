/// Port for enriching a task dispatch prompt with live project state.
///
/// Implementations call hex-nexus REST endpoints to populate architecture
/// score, relevant ADRs, and recent git changes as formatted text blocks.
/// All enrichment is best-effort — returns empty string on any failure so
/// prompt dispatch always proceeds regardless of nexus availability.
#[async_trait::async_trait]
pub trait ILiveContextPort: Send + Sync {
    /// Return a formatted context block (Markdown sections) to append to the
    /// agent prompt. `task` is used as the ADR search query; `files` are the
    /// target files declared in the workplan step.
    async fn enrich(&self, task: &str, files: &[String]) -> String;
}

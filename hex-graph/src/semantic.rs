//! Optional LLM-driven semantic edge inference.
//!
//! The engine stays network-free: instead of calling an LLM directly, it accepts
//! a `SemanticExtractor`. AST-only ("ast" mode) builds pass `NoopSemanticExtractor`;
//! "deep" mode passes an implementation that calls hex's inference path (wired in
//! hex-nexus). This keeps the engine deterministic and unit-testable.

use async_trait::async_trait;

/// A concept/relationship the extractor wants added to the graph.
#[derive(Debug, Clone)]
pub struct SemanticTriple {
    /// Source concept label (matched against existing node labels; created as a
    /// `DocConcept` if absent).
    pub source: String,
    /// Target concept label.
    pub target: String,
    /// Short relationship phrase (e.g. "depends on", "implements").
    pub relation: String,
    /// `true` if the model was confident, `false` if uncertain (→ Ambiguous).
    pub confident: bool,
}

/// Context handed to the extractor for one inference call.
#[derive(Debug, Clone)]
pub struct SemanticContext {
    /// Prose chunk (e.g. a doc file's body) to mine for relationships.
    pub text: String,
    /// Known entity labels in the graph, so the model can prefer linking to them.
    pub known_labels: Vec<String>,
    /// Source file the prose came from (for provenance).
    pub file: String,
}

#[async_trait]
pub trait SemanticExtractor: Send + Sync {
    /// Infer relationship triples from a context. Implementations should return an
    /// empty vec rather than erroring when nothing is found or the LLM is down.
    async fn infer_edges(&self, ctx: &SemanticContext) -> Vec<SemanticTriple>;
}

/// AST-only / offline extractor — infers nothing.
pub struct NoopSemanticExtractor;

#[async_trait]
impl SemanticExtractor for NoopSemanticExtractor {
    async fn infer_edges(&self, _ctx: &SemanticContext) -> Vec<SemanticTriple> {
        Vec::new()
    }
}

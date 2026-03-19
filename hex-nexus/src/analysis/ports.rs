//! Port traits for architecture analysis.
//!
//! These define the contracts that adapters (tree-sitter, filesystem) must
//! implement. The analysis use cases depend only on these traits.

use async_trait::async_trait;
use std::path::Path;

use super::domain::{
    ArchAnalysisResult, DeadExport, DependencyViolation, ExportDeclaration, ImportStatement,
    Language,
};

// ── Error Type ───────────────────────────────────────────

#[derive(Debug, thiserror::Error)]
pub enum AnalysisError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("parse error in {file}: {message}")]
    Parse { file: String, message: String },

    #[error("{0}")]
    Other(String),
}

// ── AST Port ─────────────────────────────────────────────

/// Extract imports and exports from source files.
///
/// Phase 2 (ADR-034) will provide a native tree-sitter implementation.
/// For now this trait defines the contract.
pub trait AstPort: Send + Sync {
    /// Extract all import statements from a source file.
    fn extract_imports(
        &self,
        path: &Path,
        source: &str,
        lang: Language,
    ) -> Result<Vec<ImportStatement>, AnalysisError>;

    /// Extract all export declarations from a source file.
    fn extract_exports(
        &self,
        path: &Path,
        source: &str,
        lang: Language,
    ) -> Result<Vec<ExportDeclaration>, AnalysisError>;
}

// ── Architecture Analysis Port ───────────────────────────

/// Full architecture analysis capability.
///
/// Phase 3 (ADR-034) will provide the `ArchAnalyzer` implementation
/// that composes `AstPort` with layer classification and path normalization.
#[async_trait]
pub trait ArchAnalysisPort: Send + Sync {
    /// Run full analysis: boundaries + dead exports + cycles + orphans + health score.
    async fn analyze(&self, root_path: &Path) -> Result<ArchAnalysisResult, AnalysisError>;

    /// Validate hexagonal dependency direction rules only.
    async fn validate_boundaries(
        &self,
        root_path: &Path,
    ) -> Result<Vec<DependencyViolation>, AnalysisError>;

    /// Find exports that no other file imports.
    async fn find_dead_exports(
        &self,
        root_path: &Path,
    ) -> Result<Vec<DeadExport>, AnalysisError>;

    /// Detect circular import chains via DFS.
    async fn detect_circular_deps(
        &self,
        root_path: &Path,
    ) -> Result<Vec<Vec<String>>, AnalysisError>;
}

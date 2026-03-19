//! Domain types for architecture analysis.
//!
//! Pure value objects with no external dependencies — these represent the
//! vocabulary of hexagonal architecture analysis (layers, edges, violations).

use serde::{Deserialize, Serialize};
use std::fmt;

// ── Hex Layers ───────────────────────────────────────────

/// The six canonical hexagonal architecture layers, plus special file roles.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HexLayer {
    Domain,
    Ports,
    Usecases,
    AdaptersPrimary,
    AdaptersSecondary,
    Infrastructure,
    CompositionRoot,
    EntryPoint,
    Unknown,
}

impl fmt::Display for HexLayer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Domain => write!(f, "domain"),
            Self::Ports => write!(f, "ports"),
            Self::Usecases => write!(f, "usecases"),
            Self::AdaptersPrimary => write!(f, "adapters/primary"),
            Self::AdaptersSecondary => write!(f, "adapters/secondary"),
            Self::Infrastructure => write!(f, "infrastructure"),
            Self::CompositionRoot => write!(f, "composition-root"),
            Self::EntryPoint => write!(f, "entry-point"),
            Self::Unknown => write!(f, "unknown"),
        }
    }
}

// ── Supported Languages ──────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Language {
    TypeScript,
    Go,
    Rust,
    Unknown,
}

impl Language {
    /// Detect language from file extension.
    pub fn from_path(path: &str) -> Self {
        if path.ends_with(".ts") || path.ends_with(".tsx")
            || path.ends_with(".js") || path.ends_with(".jsx")
        {
            Self::TypeScript
        } else if path.ends_with(".go") {
            Self::Go
        } else if path.ends_with(".rs") {
            Self::Rust
        } else {
            Self::Unknown
        }
    }
}

// ── Import/Export Primitives ─────────────────────────────

/// A single import statement extracted from a source file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportStatement {
    /// Project-relative path of the importing file.
    pub from_file: String,
    /// Raw import path as written in source (e.g. `../ports/index.js`, `crate::domain`).
    pub raw_path: String,
    /// Resolved project-relative path of the imported module.
    pub resolved_path: String,
    /// Source line number (1-based).
    pub line: usize,
}

/// A single exported symbol from a source file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportDeclaration {
    /// Project-relative path of the exporting file.
    pub file: String,
    /// Exported symbol name (function, type, const, etc.).
    pub name: String,
    /// Source line number (1-based).
    pub line: usize,
    /// Whether this export is annotated with `@hex:public`.
    pub hex_public: bool,
}

// ── Analysis Graph ───────────────────────────────────────

/// A directed edge in the import dependency graph.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportEdge {
    pub from_file: String,
    pub to_file: String,
    pub from_layer: HexLayer,
    pub to_layer: HexLayer,
    /// Raw import path as written in source.
    pub import_path: String,
    /// Source line number of the import statement.
    pub line: usize,
}

// ── Violation & Dead-Export Types ─────────────────────────

/// A hexagonal boundary violation: an import that crosses layers illegally.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DependencyViolation {
    pub edge: ImportEdge,
    /// Human-readable rule description (e.g. "adapters must not import from domain directly").
    pub rule: String,
}

/// An export that no other file in the project imports.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeadExport {
    pub file: String,
    pub export_name: String,
    pub line: usize,
}

// ── Full Analysis Result ─────────────────────────────────

/// Complete architecture analysis output.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArchAnalysisResult {
    pub violations: Vec<DependencyViolation>,
    pub dead_exports: Vec<DeadExport>,
    pub circular_deps: Vec<Vec<String>>,
    pub orphan_files: Vec<String>,
    pub unused_ports: Vec<String>,
    pub health_score: u8,
    pub file_count: usize,
    pub edge_count: usize,
}

impl ArchAnalysisResult {
    /// Compute health score from analysis findings.
    ///
    /// Scoring (matches TypeScript implementation):
    /// - Violations: -10 points each
    /// - Circular deps: -15 points each
    /// - Dead exports: -1 point each (capped at -20)
    /// - Unused ports: -1 point each (capped at -10)
    pub fn compute_health_score(
        violations: usize,
        circular_deps: usize,
        dead_exports: usize,
        unused_ports: usize,
    ) -> u8 {
        let penalty = (violations * 10)
            + (circular_deps * 15)
            + dead_exports.min(20)
            + unused_ports.min(10);
        100u8.saturating_sub(penalty as u8)
    }
}

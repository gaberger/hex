//! Port for ADR review and consistency validation (ADR-041).

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewFinding {
    pub severity: Severity,
    pub check: String,
    pub adr_a: String,
    pub adr_b: Option<String>,
    pub description: String,
    pub recommendation: String,
    pub file: Option<String>,
    pub line: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Severity {
    Critical,
    Warning,
    Info,
}

impl std::fmt::Display for Severity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Severity::Critical => write!(f, "CRITICAL"),
            Severity::Warning => write!(f, "WARNING"),
            Severity::Info => write!(f, "INFO"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewReport {
    pub reviewed_adr: String,
    pub timestamp: String,
    pub findings: Vec<ReviewFinding>,
    pub verdict: ReviewVerdict,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ReviewVerdict {
    Pass,
    NeedsAction,
    Blocking,
}

impl std::fmt::Display for ReviewVerdict {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ReviewVerdict::Pass => write!(f, "PASS"),
            ReviewVerdict::NeedsAction => write!(f, "NEEDS_ACTION"),
            ReviewVerdict::Blocking => write!(f, "BLOCKING"),
        }
    }
}

#[async_trait]
pub trait IAdrReviewPort: Send + Sync {
    /// Review a specific ADR against all others.
    async fn review_adr(&self, adr_path: &str) -> Result<ReviewReport, String>;
    /// Review all ADRs for cross-cutting issues.
    async fn review_all(&self) -> Result<Vec<ReviewReport>, String>;
}

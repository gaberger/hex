//! Target-app [`Objective`] — the function the application is trying to maximize
//! (or minimize) over its workload (ADR-2026-05-02-1400).
//!
//! Distinct from the *dev-process* objectives in
//! `hex-cli/src/pipeline/objectives.rs`, which gate code generation
//! ("must compile", "must pass tests"). This type describes runtime
//! application outcomes — "p95 latency < 100 ms", "cost per request < $0.001",
//! "search relevance ≥ 0.8". The two coexist; same shape, different scope.

use serde::{Deserialize, Serialize};

/// Newtype identifier for an [`Objective`].
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ObjectiveId(pub String);

/// Importance ranking for an objective.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObjectivePriority {
    Critical,
    High,
    #[default]
    Medium,
    Low,
}

/// How a measured score should compare to [`Objective::target_value`].
///
/// `WithinRange { tolerance }` means `|score - target| <= tolerance`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum ComparisonOperator {
    GreaterThan,
    GreaterThanOrEqual,
    LessThan,
    LessThanOrEqual,
    Equal,
    WithinRange { tolerance: f64 },
}

/// Lifecycle state of an objective.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObjectiveStatus {
    #[default]
    Active,
    Achieved,
    Abandoned,
    Superseded,
}

/// A target-app objective — what the application is trying to maximize/minimize.
///
/// Hierarchy: top-level objectives have `parent: None`; sub-objectives nest
/// via `parent: Some(parent_id)`. Priority ordering is independent of hierarchy.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Objective {
    pub id: ObjectiveId,
    pub name: String,
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<ObjectiveId>,
    pub priority: ObjectivePriority,
    pub target_value: f64,
    pub comparison: ComparisonOperator,
    pub unit: String,
    pub status: ObjectiveStatus,
    /// ISO 8601 timestamp when this objective was created.
    pub created_at: String,
    /// ISO 8601 timestamp of the most recent modification.
    pub updated_at: String,
}

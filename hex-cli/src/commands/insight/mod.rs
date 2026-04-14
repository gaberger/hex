//! `hex insight` — recursive insight routing (ADR-2604142345).
//!
//! The agent emits `★ Insight` blocks during non-trivial turns. These blocks
//! carry architectural observations, actionable gaps, meta-patterns, and
//! failure-mode notes — the highest-signal self-observation the system
//! produces. This module provides the *extractor* — the first half of the
//! spinal cord that turns those blocks into durable, routable artifacts.
//!
//! Phase I1 (this module) covers:
//!
//! 1. The [`Insight`] struct + its supporting enums.
//! 2. The [`extract_insights`] function that parses `★ Insight` blocks from
//!    assistant text, tolerating both structured YAML bodies and legacy prose.
//! 3. PostToolUse hook wiring lives in `hex-cli/src/commands/hook.rs` and
//!    calls into [`extract_insights`] directly.
//!
//! Later phases (I2–I5) add a classifier, router, and closure reconciler.
//! See `docs/workplans/wp-insight-routing.json` for the full plan.

pub mod extractor;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// A single extracted insight — either structured (parsed from YAML) or a
/// best-effort fallback around legacy prose.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Insight {
    /// Stable identifier. For structured insights this is author-supplied;
    /// for fallback extractions it is synthesized as
    /// `insight-<session>-<turn:03>`.
    pub id: String,
    pub kind: InsightKind,
    pub content: String,
    pub route_to: RouteTarget,
    pub estimated_tier: Tier,
    #[serde(default)]
    pub depends_on: Vec<String>,
    pub source_session: String,
    pub source_turn: usize,
    pub created_at: DateTime<Utc>,
}

/// Classification of what an insight *is*. Informs routing but is not the
/// same thing — an `ActionableGap` can still route to `Memory` if it's a
/// duplicate, etc.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum InsightKind {
    ArchitecturalObservation,
    ActionableGap,
    MetaPattern,
    FailureMode,
    Duplicate,
}

/// Where a classified insight should be materialized. Decided by the
/// classifier in I2 — the extractor only records what the author said.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RouteTarget {
    Adr,
    Workplan,
    Memory,
    DuplicateOf(String),
    Skip,
}

/// Estimated execution tier per ADR-2604120202 tiered inference routing.
/// The author estimates; later phases can override.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Tier {
    T1,
    T2,
    T3,
}

pub use extractor::extract_insights;

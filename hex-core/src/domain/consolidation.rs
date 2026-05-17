//! Consolidation domain types — value types returned by the
//! [`IConsolidationMemoryPort`] (ADR-2604261430).
//!
//! These mirror Stash's consolidation pipeline shapes (`internal/brain/`,
//! Apache-2.0). Shapes are kept narrow so the future
//! `StashSseAdapter` can projection-map cleanly.
//!
//! Adapted to hex-core conventions: timestamps as ISO 8601 `String`
//! (hex-core stays zero-deps beyond serde + thiserror + async-trait).

use serde::{Deserialize, Serialize};
use thiserror::Error;

// ── Identifiers ────────────────────────────────────────────

/// Newtype identifier for an [`Episode`].
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct EpisodeId(pub String);

/// Newtype identifier for a [`Fact`].
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct FactId(pub String);

/// Newtype identifier for a [`Contradiction`].
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ContradictionId(pub String);

// ── Episodic ───────────────────────────────────────────────

/// A single observed episode — raw input to the consolidation pipeline.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Episode {
    pub id: EpisodeId,
    pub content: String,
    pub namespace: String,
    /// ISO 8601 timestamp.
    pub created_at: String,
    /// Similarity / relevance score returned by `recall`. None for raw
    /// `remember` reads.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub score: Option<f32>,
}

// ── Knowledge ──────────────────────────────────────────────

/// A consolidated, confidence-scored fact synthesized from one or more episodes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Fact {
    pub id: FactId,
    pub content: String,
    pub confidence: f32,
    pub namespace: String,
    pub sources: Vec<EpisodeId>,
    /// ISO 8601 timestamp of the most recent observation supporting this fact.
    pub last_seen_at: String,
}

/// A subject-predicate-object edge between two entities.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Relationship {
    pub id: String,
    pub subject: String,
    pub predicate: String,
    pub object: String,
    pub confidence: f32,
}

/// A pair of facts the consolidation pipeline detected as conflicting.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Contradiction {
    pub id: ContradictionId,
    pub fact_a: FactId,
    pub fact_b: FactId,
    /// ISO 8601 timestamp the contradiction was detected.
    pub detected_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolution: Option<String>,
}

// ── Causal graph ───────────────────────────────────────────

/// Direction of traversal when tracing a causal chain.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CausalDirection {
    Forward,
    Backward,
}

/// A directed causal edge between two facts.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CausalLink {
    pub cause: FactId,
    pub effect: FactId,
    pub confidence: f32,
}

/// The chain returned by `trace_causal_chain`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CausalChain {
    pub root: FactId,
    pub links: Vec<CausalLink>,
    pub max_depth_reached: bool,
}

// ── Consolidation report ───────────────────────────────────

/// Summary returned by `consolidate`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConsolidationReport {
    pub stages_run: Vec<String>,
    pub episodes_processed: u32,
    pub facts_created: u32,
    pub contradictions_found: u32,
    pub took_ms: u64,
}

// ── Errors ─────────────────────────────────────────────────

/// Errors surfaced by [`super::super::ports::consolidation_memory::IConsolidationMemoryPort`]
/// implementations.
#[derive(Debug, Error)]
pub enum ConsolidationError {
    #[error("not implemented")]
    NotImplemented,

    #[error("backend error: {0}")]
    Backend(String),

    #[error("invalid namespace: {0}")]
    InvalidNamespace(String),

    #[error("reasoner error: {0}")]
    Reasoner(String),

    #[error("operation timed out")]
    Timeout,
}

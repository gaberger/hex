//! Consolidation memory port — contract for stash-style persistent memory
//! (ADR-2026-04-26-1430).
//!
//! Implemented by `NoopConsolidationAdapter` (always-available fallback) and
//! `StashSseAdapter` (proxies to a stash sidecar). Lives alongside, not
//! inside, the existing `IHexFloMemoryStatePort` (which is the KV
//! coordination memory).

use async_trait::async_trait;

use crate::domain::consolidation::{
    CausalChain, CausalDirection, ConsolidationError, ConsolidationReport, Contradiction,
    Episode, EpisodeId, Fact, Relationship,
};

/// The consolidation memory port — exposes episodic I/O, the consolidate
/// trigger, knowledge queries, contradiction handling, and causal-chain
/// tracing. Narrowed from stash's 28 MCP tools to the high-value subset.
#[async_trait]
pub trait IConsolidationMemoryPort: Send + Sync {
    // ── Episodic I/O ───────────────────────────────────────
    async fn remember(
        &self,
        content: &str,
        namespace: &str,
    ) -> Result<EpisodeId, ConsolidationError>;

    async fn recall(
        &self,
        query: &str,
        namespaces: &[String],
        limit: u32,
    ) -> Result<Vec<Episode>, ConsolidationError>;

    async fn forget(
        &self,
        about: &str,
        namespaces: &[String],
    ) -> Result<u32, ConsolidationError>;

    // ── Consolidation trigger ──────────────────────────────
    /// Idempotent — only processes new data since the last run, per
    /// stash's `internal/brain/consolidate.go` behavior.
    async fn consolidate(
        &self,
        namespaces: &[String],
    ) -> Result<ConsolidationReport, ConsolidationError>;

    // ── Knowledge queries ──────────────────────────────────
    async fn query_facts(
        &self,
        query: &str,
        namespace: &str,
    ) -> Result<Vec<Fact>, ConsolidationError>;

    async fn query_relationships(
        &self,
        entity: &str,
        namespace: &str,
    ) -> Result<Vec<Relationship>, ConsolidationError>;

    // ── Contradictions ─────────────────────────────────────
    async fn list_contradictions(
        &self,
        namespace: &str,
    ) -> Result<Vec<Contradiction>, ConsolidationError>;

    async fn resolve_contradiction(
        &self,
        id: &str,
        resolution: &str,
    ) -> Result<(), ConsolidationError>;

    // ── Causal graph ───────────────────────────────────────
    async fn trace_causal_chain(
        &self,
        fact_id: &str,
        direction: CausalDirection,
        max_depth: u8,
    ) -> Result<CausalChain, ConsolidationError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Returns `ConsolidationError::NotImplemented` from every method.
    /// Compile-time guard that the trait stays dyn-safe across future
    /// signature changes.
    struct DummyConsolidation;

    #[async_trait]
    impl IConsolidationMemoryPort for DummyConsolidation {
        async fn remember(
            &self,
            _content: &str,
            _namespace: &str,
        ) -> Result<EpisodeId, ConsolidationError> {
            Err(ConsolidationError::NotImplemented)
        }
        async fn recall(
            &self,
            _query: &str,
            _namespaces: &[String],
            _limit: u32,
        ) -> Result<Vec<Episode>, ConsolidationError> {
            Err(ConsolidationError::NotImplemented)
        }
        async fn forget(
            &self,
            _about: &str,
            _namespaces: &[String],
        ) -> Result<u32, ConsolidationError> {
            Err(ConsolidationError::NotImplemented)
        }
        async fn consolidate(
            &self,
            _namespaces: &[String],
        ) -> Result<ConsolidationReport, ConsolidationError> {
            Err(ConsolidationError::NotImplemented)
        }
        async fn query_facts(
            &self,
            _query: &str,
            _namespace: &str,
        ) -> Result<Vec<Fact>, ConsolidationError> {
            Err(ConsolidationError::NotImplemented)
        }
        async fn query_relationships(
            &self,
            _entity: &str,
            _namespace: &str,
        ) -> Result<Vec<Relationship>, ConsolidationError> {
            Err(ConsolidationError::NotImplemented)
        }
        async fn list_contradictions(
            &self,
            _namespace: &str,
        ) -> Result<Vec<Contradiction>, ConsolidationError> {
            Err(ConsolidationError::NotImplemented)
        }
        async fn resolve_contradiction(
            &self,
            _id: &str,
            _resolution: &str,
        ) -> Result<(), ConsolidationError> {
            Err(ConsolidationError::NotImplemented)
        }
        async fn trace_causal_chain(
            &self,
            _fact_id: &str,
            _direction: CausalDirection,
            _max_depth: u8,
        ) -> Result<CausalChain, ConsolidationError> {
            Err(ConsolidationError::NotImplemented)
        }
    }

    #[test]
    fn consolidation_memory_port_is_dyn_safe() {
        let dummy = DummyConsolidation;
        let _erased: &dyn IConsolidationMemoryPort = &dummy;
    }
}

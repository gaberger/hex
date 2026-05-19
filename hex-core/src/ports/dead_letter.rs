//! IDeadLetterPort — contract for runtime quarantine + replay (ADR-2605190900 P2.2).
//!
//! Long-running components (dispatcher, retry loops, watchdog) call this
//! port when a brain-task exceeds its retry budget. The production impl
//! lives in `hex-nexus/src/adapters/spacetime_dead_letter.rs`.
//!
//! Trait shape mirrors the rest of `hex-core/src/ports/`:
//! - `#[async_trait]` so impls can do I/O.
//! - `Send + Sync` for `Arc<dyn IDeadLetterPort>`.
//! - Object-safe (no generic methods).

use async_trait::async_trait;

use crate::domain::dead_letter::DeadLetterRecord;

/// Error surface for dead-letter operations.
#[derive(Debug, thiserror::Error)]
pub enum DeadLetterError {
    #[error("dead-letter backend unreachable: {0}")]
    BackendUnreachable(String),
    #[error("dead-letter unknown task_id: {0}")]
    NotFound(String),
    #[error("dead-letter error: {0}")]
    Other(String),
}

/// Contract every long-running component implements (via an adapter) so
/// quarantined work is durably recorded and replayable.
#[async_trait]
pub trait IDeadLetterPort: Send + Sync {
    /// Quarantine a brain-task. Idempotent on duplicate task_id —
    /// updates the existing row, preserving first_failed_at.
    async fn record(
        &self,
        task_id: &str,
        kind: &str,
        payload: &str,
        last_error: &str,
        attempt_count: u32,
        original_priority: i32,
    ) -> Result<(), DeadLetterError>;

    /// Read the full quarantine queue, newest-failure-first.
    async fn list(&self) -> Result<Vec<DeadLetterRecord>, DeadLetterError>;

    /// Operator-driven replay — removes the row + returns its shape so
    /// the caller can re-enqueue at the dispatcher API. Returns `Ok(None)`
    /// on unknown task_id so the dashboard's Replay button is safe to
    /// double-click.
    async fn replay(&self, task_id: &str) -> Result<Option<DeadLetterRecord>, DeadLetterError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[allow(dead_code)]
    fn trait_object_safe(port: std::sync::Arc<dyn IDeadLetterPort>) -> std::sync::Arc<dyn IDeadLetterPort> {
        port
    }
}

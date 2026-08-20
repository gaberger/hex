//! IHeartbeatPort — contract for runtime liveness signals (ADR-2026-05-19-0900 P1.1).
//!
//! Long-running components (sched daemon, nexus server, workplan_executor,
//! hex-agent worker, etc.) implement this port via a secondary adapter so
//! they can publish heartbeat rows and supervisors can query for missed
//! beats. The port is **pure trait** — no I/O, no STDB awareness; the
//! production impl lives in `hex-nexus/src/adapters/secondary/spacetime_heartbeat.rs`.
//!
//! Trait shape rules (mirrors the rest of `hex-core/src/ports/`):
//! - `#[async_trait]` so impls can do I/O.
//! - `Send + Sync` bound so adapters work behind `Arc<dyn IHeartbeatPort>`.
//! - Object-safe (no generics on the trait methods) so we can stash
//!   one in a state struct.

use std::time::Duration;

use async_trait::async_trait;

use crate::domain::heartbeat::{HeartbeatStatus, WorkerHeartbeat};

/// Error surface for heartbeat operations. Concrete adapters map their
/// transport-level errors into this shape so the trait stays portable.
#[derive(Debug, thiserror::Error)]
pub enum HeartbeatError {
    /// The registry backend (STDB, in-memory, file) was unreachable.
    #[error("heartbeat backend unreachable: {0}")]
    BackendUnreachable(String),
    /// The supplied identifier conflicted with an existing row in a way
    /// the adapter couldn't resolve (e.g. dup `worker_id` with a different
    /// `pool_id`).
    #[error("heartbeat identity conflict: {0}")]
    IdentityConflict(String),
    /// Any other adapter-internal failure — never silently swallowed.
    #[error("heartbeat error: {0}")]
    Other(String),
}

/// Contract every long-running component implements (via an adapter) so
/// the supervision layer can see who is alive.
#[async_trait]
pub trait IHeartbeatPort: Send + Sync {
    /// First call a component makes — claims a row in `worker_process`.
    /// Subsequent `beat` calls update `last_heartbeat_at` on the same row.
    /// Returns the `worker_id` the registry assigned (typically what the
    /// caller supplied; adapters may augment with host/pid suffixes).
    async fn register(
        &self,
        worker_id: &str,
        pool_id: &str,
        role: &str,
        pid: u32,
        host: &str,
    ) -> Result<String, HeartbeatError>;

    /// Periodic tick. Components should call this every `ttl / 3` or so —
    /// the supervisor's miss-detection threshold is `ttl`, so beating
    /// less often than that risks false positives.
    async fn beat(
        &self,
        worker_id: &str,
        status: HeartbeatStatus,
        evidence: Option<&str>,
    ) -> Result<(), HeartbeatError>;

    /// Graceful shutdown — removes the row so the supervisor doesn't
    /// wait for the TTL. Adapters MUST be idempotent: calling
    /// `deregister` twice or on an unknown id is not an error.
    async fn deregister(&self, worker_id: &str) -> Result<(), HeartbeatError>;

    /// Query: every worker in `role` whose `last_heartbeat_at` is newer
    /// than `now - ttl`. Used by the dispatcher to refuse-to-publish when
    /// no consumer exists, and by `hex doctor liveness`.
    async fn list_alive(
        &self,
        role: &str,
        ttl: Duration,
    ) -> Result<Vec<WorkerHeartbeat>, HeartbeatError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Compile-time check: trait is object-safe and can be stored as
    /// `Arc<dyn IHeartbeatPort>`. If this stops compiling, someone added
    /// a generic-method to the trait — back it out.
    #[allow(dead_code)]
    fn trait_object_safe(port: std::sync::Arc<dyn IHeartbeatPort>) -> std::sync::Arc<dyn IHeartbeatPort> {
        port
    }
}

//! IWorkerPoolPort — contract for the dispatcher's "is there a consumer?"
//! gate (ADR-2605190900 §1 + P3.4).
//!
//! Long-running dispatchers call `ensure_consumer(role)` before publishing
//! work. Production impl wraps the `worker_process` STDB table; tests use
//! an in-memory stub.
//!
//! Trait shape matches the rest of `hex-core/src/ports/`:
//! - `#[async_trait]` for I/O impls
//! - `Send + Sync` for `Arc<dyn IWorkerPoolPort>`
//! - Object-safe (no generic methods)

use std::time::Duration;

use async_trait::async_trait;

use crate::domain::worker_pool::ConsumerStatus;

#[derive(Debug, thiserror::Error)]
pub enum WorkerPoolError {
    #[error("worker-pool backend unreachable: {0}")]
    BackendUnreachable(String),
    #[error("worker-pool error: {0}")]
    Other(String),
}

#[async_trait]
pub trait IWorkerPoolPort: Send + Sync {
    /// Returns the current consumer status for `role`. `ttl` is the
    /// staleness threshold — a worker_process row whose `last_heartbeat`
    /// is older than `now - ttl` does NOT count toward Alive.
    async fn ensure_consumer(
        &self,
        role: &str,
        ttl: Duration,
    ) -> Result<ConsumerStatus, WorkerPoolError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[allow(dead_code)]
    fn trait_object_safe(port: std::sync::Arc<dyn IWorkerPoolPort>) -> std::sync::Arc<dyn IWorkerPoolPort> {
        port
    }
}

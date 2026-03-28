//! ISandboxPort — contract for spawning and managing sandbox containers.

use async_trait::async_trait;
use crate::domain::sandbox::{SandboxConfig, SandboxError, SpawnResult};

/// Port for managing the lifecycle of sandbox containers (Docker / microVM).
///
/// Implementations live in adapters/secondary — never import adapters here.
#[async_trait]
pub trait ISandboxPort: Send + Sync {
    /// Spawn a new sandbox container for the given configuration.
    async fn spawn(&self, config: SandboxConfig) -> Result<SpawnResult, SandboxError>;

    /// Stop and remove a running container by its ID.
    async fn stop(&self, container_id: &str) -> Result<(), SandboxError>;

    /// Return the current status string for a container (e.g. `"running"`,
    /// `"exited"`, `"not found"`).
    async fn status(&self, container_id: &str) -> Result<String, SandboxError>;

    /// List all sandboxes currently tracked by this adapter.
    async fn list(&self) -> Result<Vec<SpawnResult>, SandboxError>;
}

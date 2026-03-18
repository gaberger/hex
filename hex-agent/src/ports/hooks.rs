use crate::domain::{HookConfig, HookEvent, HookResult};
use async_trait::async_trait;

/// Port for executing lifecycle hooks.
#[async_trait]
pub trait HookRunnerPort: Send + Sync {
    /// Load hook configuration from settings.
    async fn load_config(&self, settings_path: &str) -> Result<HookConfig, HookError>;

    /// Execute all hooks for a given event.
    /// Returns results for each hook (success or failure).
    async fn run_hooks(
        &self,
        config: &HookConfig,
        event: &HookEvent,
        env: &[(&str, &str)],
    ) -> Vec<HookResult>;
}

#[derive(Debug, thiserror::Error)]
pub enum HookError {
    #[error("Failed to load hook config: {0}")]
    LoadError(String),
    #[error("Hook execution failed: {0}")]
    ExecutionError(String),
}

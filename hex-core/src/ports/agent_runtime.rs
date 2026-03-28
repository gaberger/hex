//! IAgentRuntimePort — contract for executing agent tasks inside a sandbox.

use async_trait::async_trait;
use crate::domain::sandbox::{AgentTask, SandboxError, ToolResult};

/// Port for dispatching tasks to an agent runtime and collecting results.
///
/// Implementations live in adapters/secondary — never import adapters here.
#[async_trait]
pub trait IAgentRuntimePort: Send + Sync {
    /// Dispatch a task to the agent runtime and wait for the result.
    async fn execute_task(&self, task: AgentTask) -> Result<ToolResult, SandboxError>;

    /// Report that a task has completed with the given result summary.
    async fn report_completion(&self, task_id: &str, result: &str) -> Result<(), SandboxError>;
}

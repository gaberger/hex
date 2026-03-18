use crate::domain::{ToolCall, ToolResult};
use async_trait::async_trait;

/// Port for executing tools on behalf of the LLM.
///
/// When the Anthropic API returns tool_use blocks, the conversation loop
/// calls this port to execute each tool and feed results back.
#[async_trait]
pub trait ToolExecutorPort: Send + Sync {
    /// Execute a tool call and return the result.
    async fn execute(&self, call: &ToolCall) -> ToolResult;

    /// Check if a tool name is available.
    fn has_tool(&self, name: &str) -> bool;

    /// Get the working directory for file operations.
    fn working_dir(&self) -> &str;
}

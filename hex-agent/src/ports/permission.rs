use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PermissionDecision {
    Allow,
    Deny { reason: String },
    Pending { reason: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolPermission {
    pub tool_name: String,
    pub args: serde_json::Value,
    pub decision: PermissionDecision,
}

#[async_trait::async_trait]
pub trait PermissionPort: Send + Sync {
    async fn check_permission(&self, tool_name: &str, args: &serde_json::Value) -> ToolPermission;
    async fn check_batch(&self, tools: Vec<(&str, &serde_json::Value)>) -> Vec<ToolPermission>;
}
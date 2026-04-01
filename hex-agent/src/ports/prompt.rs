use crate::domain::context::{AgentRole, ServiceTemplate, SystemTemplate, ToolTemplate};
use async_trait::async_trait;

#[async_trait]
pub trait PromptPort: Send + Sync {
    async fn build_system_prompt(
        &self,
        role: AgentRole,
        template: SystemTemplate,
    ) -> Result<String, ContextError>;

    async fn build_tool_prompt(
        &self,
        tool: ToolTemplate,
    ) -> Result<String, ContextError>;

    async fn build_service_prompt(
        &self,
        service: ServiceTemplate,
    ) -> Result<String, ContextError>;

    /// Build and cache the full composed prompt for a role (all sections joined).
    /// Variable substitution is intentionally excluded — apply `ContextBuilder`
    /// with live `ContextVariables` on the returned template string at the
    /// use-case layer.
    async fn build_composed_prompt(&self, role: AgentRole) -> Result<String, ContextError>;

    async fn reload_templates(&self) -> Result<(), ContextError>;
}

#[derive(Debug, thiserror::Error)]
pub enum ContextError {
    #[error("Template not found: {0}")]
    TemplateNotFound(String),
    #[error("Variable missing: {0}")]
    VariableMissing(String),
    #[error("Template compilation failed: {0}")]
    CompilationFailed(String),
    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),
}
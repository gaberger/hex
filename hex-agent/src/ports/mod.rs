pub mod anthropic;
pub mod conversation;
pub mod context;
pub mod tools;
pub mod skills;
pub mod hooks;
pub mod agents;
pub mod workplan;
pub mod hub;

pub use anthropic::AnthropicPort;
pub use context::ContextManagerPort;
pub use tools::ToolExecutorPort;
pub use skills::SkillLoaderPort;
pub use hooks::HookRunnerPort;
pub use agents::AgentLoaderPort;
pub use workplan::WorkplanPort;
pub use hub::HubClientPort;

// Re-export domain types that adapters need (so adapters import from ports, not domain)
pub use crate::domain::{
    AgentDefinition, AgentConstraints,
    ConversationState, StopReason,
    TokenBudget, TokenUsage,
};

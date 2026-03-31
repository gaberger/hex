pub mod anthropic;
pub mod conversation;
pub mod context;
pub mod tools;
pub mod skills;
pub mod hooks;
pub mod agents;
pub mod workplan;
pub mod hub;
pub mod rl;
pub mod secret_broker;
pub mod inference;
pub mod rate_limiter;
pub mod batch;
pub mod token_metrics;
pub mod preflight;
pub mod mcp_client;
pub mod output_analyzer;
pub mod permission;

pub use anthropic::AnthropicPort;
pub use context::ContextManagerPort;
pub use tools::ToolExecutorPort;
pub use skills::SkillLoaderPort;
pub use hooks::HookRunnerPort;
pub use agents::AgentLoaderPort;
pub use workplan::WorkplanPort;
pub use hub::HubClientPort;
pub use secret_broker::SecretBrokerPort;
pub mod inference_client;
pub use inference::InferenceDiscoveryPort;
pub use inference_client::InferenceClientPort;
pub use rate_limiter::RateLimiterPort;
pub use batch::BatchPort;
pub use token_metrics::TokenMetricsPort;
pub use preflight::PreflightPort;
pub mod command_session;
pub use command_session::IBatchExecutionPort;

// Re-export domain types that adapters need (so adapters import from ports, not domain)
#[allow(unused_imports)]
pub use crate::domain::{
    AgentDefinition, AgentConstraints,
    ContentBlock, Message, Role,
    ConversationState, StopReason,
    TokenBudget, TokenUsage,
    ToolCall, ToolResult, ToolDefinition,
    Skill, SkillTrigger, SkillManifest,
    Hook, HookEvent, HookConfig, HookResult,
    ApiRequestOptions, RateLimitHeaders,
    RateLimitState, ApiMetricsSnapshot, CacheMetrics,
};

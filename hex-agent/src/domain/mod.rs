// ── Shared types from hex-core (single source of truth) ──────────
pub use hex_core::domain::agents;
pub use hex_core::domain::api_optimization;
pub use hex_core::domain::hooks;
pub use hex_core::domain::messages;
pub use hex_core::domain::secret_grant;
pub use hex_core::domain::tokens;
pub use hex_core::domain::workplan;

// ── Agent-specific modules (NOT shared with hex-nexus) ───────────
pub mod hex_knowledge;
pub mod mcp;
pub mod output_score;
pub mod pricing;
pub mod skills; // Re-exports hex-core skill types + adds match_input()
pub mod tools; // Re-exports hex-core tool types + adds builtin_tools()

// Re-export core types for convenience
pub use agents::{AgentConstraints, AgentDefinition, AgentMetrics, AgentStatus};
pub use api_optimization::{
    ApiMetricsSnapshot, ApiRequestOptions, BatchRequest, BatchStatus, CacheMetrics,
    RateLimitHeaders, RateLimitState, ThinkingConfig, WorkloadClass,
};
pub use hex_core::domain::skills::{Skill, SkillManifest, SkillTrigger};
pub use hooks::{Hook, HookConfig, HookEvent, HookResult};
pub use messages::{ContentBlock, ConversationState, Message, Role, StopReason};
pub use tokens::{TokenBudget, TokenPartition, TokenUsage};
pub use tools::{ToolCall, ToolDefinition, ToolInputSchema, ToolResult};
pub use workplan::{PhaseGate, TaskStatus, Workplan, WorkplanPhase, WorkplanTask};

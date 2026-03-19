// ── Shared types from hex-core (single source of truth) ──────────
pub use hex_core::domain::messages;
pub use hex_core::domain::tokens;
pub use hex_core::domain::agents;
pub use hex_core::domain::hooks;
pub use hex_core::domain::workplan;
pub use hex_core::domain::secret_grant;
pub use hex_core::domain::api_optimization;

// ── Agent-specific modules (NOT shared with hex-nexus) ───────────
pub mod tools;          // Re-exports hex-core tool types + adds builtin_tools()
pub mod skills;         // Re-exports hex-core skill types + adds match_input()
pub mod hex_knowledge;
pub mod mcp;
pub mod output_score;

// Re-export core types for convenience
pub use messages::{Message, Role, ContentBlock, StopReason, ConversationState};
pub use tokens::{TokenBudget, TokenPartition, TokenUsage};
pub use tools::{ToolCall, ToolResult, ToolDefinition, ToolInputSchema};
pub use agents::{AgentDefinition, AgentConstraints, AgentMetrics, AgentStatus};
pub use hex_core::domain::skills::{Skill, SkillTrigger, SkillManifest};
pub use hooks::{Hook, HookEvent, HookResult, HookConfig};
pub use workplan::{Workplan, WorkplanPhase, WorkplanTask, TaskStatus, PhaseGate};
pub use api_optimization::{
    WorkloadClass, ThinkingConfig, ApiRequestOptions,
    CacheMetrics, RateLimitState, RateLimitHeaders,
    BatchStatus, BatchRequest, ApiMetricsSnapshot,
};

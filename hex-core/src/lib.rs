//! hex-core — Shared domain types and port traits for the hex framework.
//!
//! This crate is the single source of truth for types used across hex-nexus,
//! hex-agent, hex-chat, and hex-cli. It has zero runtime dependencies beyond
//! serde, thiserror, and async-trait.
//!
//! # Architecture
//!
//! ```text
//! hex-core (this crate)
//!   ├── domain/     — Value objects and entities (pure data, no I/O)
//!   ├── ports/      — Trait definitions (contracts between layers)
//!   └── rules/      — Hex architecture enforcement logic
//! ```

pub mod domain;
pub mod ports;
pub mod rules;

/// Re-export commonly used types at the crate root.
pub use domain::agents::{AgentConstraints, AgentDefinition, AgentMetrics};
pub use domain::messages::{ContentBlock, ConversationState, Message, Role, StopReason};
pub use domain::tokens::{TokenBudget, TokenPartition, TokenUsage};
pub use domain::tools::{ToolCall, ToolDefinition, ToolInputSchema, ToolResult};
pub use domain::workplan::{PhaseGate, TaskStatus, Workplan, WorkplanPhase, WorkplanTask};

//! hex-core — Shared domain types and port traits for the hex framework.
//!
//! The contract surface every other crate depends on, and the gravity centre
//! of founding goal G3: it pulls only zero-runtime crates, so nothing below it
//! can bleed a runtime concern upward.
//!
//! ```text
//! hex-core (this crate)
//!   ├── domain/     — value objects (pure data, no I/O)
//!   ├── ports/      — trait definitions (contracts between layers)
//!   └── rules/      — hexagonal enforcement logic
//! ```
//!
//! Trimmed from 8,257 lines by ADR-2608241500 P7. What went: the state,
//! coordination, heartbeat, worker-pool, dead-letter, secret, sandbox, brain,
//! agent-runtime and experiment ports, and the domain types that only those
//! ports named. Every one of them existed to describe a fleet of agents
//! sharing a database. What stays is what a crate outside this one actually
//! uses — checked, not assumed.

pub mod domain;
pub mod ports;
pub mod quantization;
pub mod resource_governor;
pub mod rules;
pub mod validation;

/// Re-exports for the types callers reach for most.
pub use quantization::QuantizationLevel;
pub use domain::messages::{ContentBlock, ConversationState, Message, Role, StopReason};
pub use domain::tools::{ToolCall, ToolDefinition, ToolInputSchema, ToolResult};

//! hex-core — Shared domain types and port traits for the hex framework.
//!
//! This crate is the single source of truth for types used across hex-nexus,
//! hex-agent, and hex-cli. It has zero runtime dependencies beyond
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

// ── Infrastructure Constants ──────────────────────────────
// Shared across hex-cli, hex-nexus, and hex-agent to prevent string drift.

/// Canonical SpacetimeDB health-check endpoint path.
/// All code that pings SpacetimeDB MUST use this constant — never hardcode the path.
/// See ADR rule `adr-039-no-stale-ping` for enforcement.
/// Updated for SpacetimeDB v2.0.5+ which moved /database/ping → /v1/ping.
pub const SPACETIMEDB_PING_PATH: &str = "/v1/ping";

/// Default SpacetimeDB host URL.
/// Port 3033 chosen to avoid conflicts with common dev servers (Next.js, Rails on 3000).
pub const SPACETIMEDB_DEFAULT_HOST: &str = "http://127.0.0.1:3033";

/// Re-export commonly used types at the crate root.
pub use domain::agents::{AgentConstraints, AgentDefinition, AgentMetrics};
pub use domain::messages::{ContentBlock, ConversationState, Message, Role, StopReason};
pub use domain::tokens::{TokenBudget, TokenPartition, TokenUsage};
pub use domain::tools::{ToolCall, ToolDefinition, ToolInputSchema, ToolResult};
pub use domain::workplan::{PhaseGate, TaskStatus, Workplan, WorkplanPhase, WorkplanTask};

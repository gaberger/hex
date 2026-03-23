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

// ── SpacetimeDB Module Database Names ─────────────────────
// Each WASM module publishes to its own database (ADR-2603231500).
// hexflo-coordination → "hex" (backward compat), all others → directory name.

/// Database name for the core coordination module (backward-compatible).
pub const STDB_DATABASE_CORE: &str = "hex";

/// Module-to-database mapping. Index matches MODULE_TIERS order in spacetime_launcher.
/// Format: (module_directory_name, database_name)
pub const STDB_MODULE_DATABASES: &[(&str, &str)] = &[
    // Tier 0: Foundation
    ("hexflo-coordination", "hex"),          // mega-module, backward compat
    ("agent-registry", "agent-registry"),
    ("fleet-state", "fleet-state"),
    ("file-lock-manager", "file-lock-manager"),
    // Tier 1: Services
    ("inference-gateway", "inference-gateway"),
    ("inference-bridge", "inference-bridge"),
    ("secret-grant", "secret-grant"),
    ("architecture-enforcer", "architecture-enforcer"),
    // Tier 2: Workflows
    ("workplan-state", "workplan-state"),
    ("skill-registry", "skill-registry"),
    ("hook-registry", "hook-registry"),
    ("agent-definition-registry", "agent-definition-registry"),
    // Tier 3: Coordination
    ("chat-relay", "chat-relay"),
    ("rl-engine", "rl-engine"),
    ("hexflo-lifecycle", "hexflo-lifecycle"),
    ("hexflo-cleanup", "hexflo-cleanup"),
    ("conflict-resolver", "conflict-resolver"),
    ("test-results", "test-results"),
];

/// Look up the database name for a module by its directory name.
/// Returns the module name itself if not found (convention: dir name = db name).
pub fn stdb_database_for_module(module_name: &str) -> &str {
    STDB_MODULE_DATABASES
        .iter()
        .find(|(name, _)| *name == module_name)
        .map(|(_, db)| *db)
        .unwrap_or(module_name)
}

/// Re-export commonly used types at the crate root.
pub use domain::agents::{AgentConstraints, AgentDefinition, AgentMetrics};
pub use domain::messages::{ContentBlock, ConversationState, Message, Role, StopReason};
pub use domain::tokens::{TokenBudget, TokenPartition, TokenUsage};
pub use domain::tools::{ToolCall, ToolDefinition, ToolInputSchema, ToolResult};
pub use domain::workplan::{PhaseGate, TaskStatus, Workplan, WorkplanPhase, WorkplanTask};

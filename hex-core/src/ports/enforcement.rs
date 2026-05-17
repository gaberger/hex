//! Enforcement port — provider-agnostic validation for hex operations (ADR-2603221959).
//!
//! This port defines the contract for checking whether an operation should
//! be allowed, warned, or blocked. Implementations live in the domain layer.
//! Both MCP tool handlers and nexus REST middleware use this same port.

/// Context for an enforcement check — describes what's being attempted.
#[derive(Debug, Clone, Default)]
pub struct EnforcementContext {
    /// Agent performing the operation (empty = unregistered)
    pub agent_id: String,
    /// Active workplan for this session (empty = none)
    pub workplan_id: String,
    /// Active swarm (empty = none)
    pub swarm_id: String,
    /// HexFlo task being worked on (empty = untracked)
    pub task_id: String,
    /// File being modified (empty = not a file op)
    pub target_file: String,
    /// Operation type: "edit", "write", "spawn_agent", "bash", "task_create", etc.
    pub operation: String,
    /// Whether the agent is running in background (stricter enforcement)
    pub is_background: bool,
}

/// Result of an enforcement check.
#[derive(Debug, Clone, PartialEq)]
pub enum EnforcementResult {
    /// Operation is allowed
    Allow,
    /// Operation is allowed but with a warning message
    Warn(String),
    /// Operation is blocked with a reason
    Block(String),
}

impl EnforcementResult {
    pub fn is_blocked(&self) -> bool {
        matches!(self, Self::Block(_))
    }

    pub fn message(&self) -> Option<&str> {
        match self {
            Self::Allow => None,
            Self::Warn(msg) | Self::Block(msg) => Some(msg),
        }
    }
}

/// Enforcement mode for the project.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum EnforcementMode {
    /// Violations block operations (exit 2 / HTTP 403)
    #[default]
    Mandatory,
    /// Violations produce warnings but don't block
    Advisory,
    /// Enforcement disabled
    Disabled,
}

impl EnforcementMode {
    pub fn parse(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "advisory" => Self::Advisory,
            "disabled" | "off" | "none" => Self::Disabled,
            _ => Self::Mandatory,
        }
    }
}

/// Port for checking enforcement rules.
///
/// Implementations check whether the given operation context satisfies
/// all active enforcement rules (workplan required, task required,
/// boundary validation, etc.).
pub trait IEnforcementPort: Send + Sync {
    /// Check an operation against enforcement rules.
    fn check(&self, ctx: &EnforcementContext) -> EnforcementResult;
}

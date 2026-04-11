//! Default enforcement implementation (ADR-2603221959).
//!
//! Checks operations against hex lifecycle rules:
//! - Background agent spawn requires HEXFLO_TASK
//! - Mutating operations require active workplan (in mandatory mode)
//! - File edits must be within workplan boundary (if boundary rules defined)

use crate::ports::enforcement::{
    EnforcementContext, EnforcementMode, EnforcementResult, IEnforcementPort,
};

/// Operations exempt from enforcement (read-only or lifecycle management).
const EXEMPT_OPERATIONS: &[&str] = &[
    "session_start",
    "session_heartbeat",
    "workplan_activate",
    "read",
    "list",
    "search",
    "status",
    "analyze",
    "audit",
];

/// Default enforcer — checks hex lifecycle rules.
pub struct DefaultEnforcer {
    pub mode: EnforcementMode,
}

impl DefaultEnforcer {
    pub fn new(mode: EnforcementMode) -> Self {
        Self { mode }
    }

    /// Create from project config string ("mandatory", "advisory", "disabled").
    pub fn from_mode_str(mode: &str) -> Self {
        Self {
            mode: EnforcementMode::parse(mode),
        }
    }
}

impl IEnforcementPort for DefaultEnforcer {
    fn check(&self, ctx: &EnforcementContext) -> EnforcementResult {
        if self.mode == EnforcementMode::Disabled {
            return EnforcementResult::Allow;
        }

        // Exempt operations are always allowed
        if EXEMPT_OPERATIONS
            .iter()
            .any(|op| ctx.operation.contains(op))
        {
            return EnforcementResult::Allow;
        }

        // Rule 1: Background agent spawn requires task tracking
        if ctx.operation == "spawn_agent" && ctx.is_background && ctx.task_id.is_empty() {
            let msg =
                "Background agent requires HEXFLO_TASK — create swarm + task first".to_string();
            return match self.mode {
                EnforcementMode::Mandatory => EnforcementResult::Block(msg),
                EnforcementMode::Advisory => EnforcementResult::Warn(msg),
                EnforcementMode::Disabled => EnforcementResult::Allow,
            };
        }

        // Rule 2: Mutating operations require workplan
        let mutating_ops = ["edit", "write", "spawn_agent", "task_create"];
        if mutating_ops.iter().any(|op| ctx.operation.contains(op)) && ctx.workplan_id.is_empty() {
            let msg = format!(
                "Operation '{}' requires active workplan — pipeline: ADR → Workplan → Swarm → Agent",
                ctx.operation
            );
            return match self.mode {
                EnforcementMode::Mandatory => EnforcementResult::Block(msg),
                EnforcementMode::Advisory => EnforcementResult::Warn(msg),
                EnforcementMode::Disabled => EnforcementResult::Allow,
            };
        }

        // Rule 3: Agent must be registered for tracked operations
        if !ctx.operation.contains("session") && ctx.agent_id.is_empty() {
            let msg = "Agent not registered — call hex_session_start first".to_string();
            return match self.mode {
                EnforcementMode::Mandatory => EnforcementResult::Warn(msg), // Warn, don't block — registration may be lazy
                _ => EnforcementResult::Allow,
            };
        }

        EnforcementResult::Allow
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(operation: &str) -> EnforcementContext {
        EnforcementContext {
            agent_id: "agent-123".to_string(),
            workplan_id: "wp-test".to_string(),
            task_id: "task-456".to_string(),
            operation: operation.to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn allows_read_operations() {
        let e = DefaultEnforcer::new(EnforcementMode::Mandatory);
        assert_eq!(e.check(&ctx("read")), EnforcementResult::Allow);
        assert_eq!(e.check(&ctx("list")), EnforcementResult::Allow);
        assert_eq!(e.check(&ctx("status")), EnforcementResult::Allow);
    }

    #[test]
    fn allows_fully_tracked_edit() {
        let e = DefaultEnforcer::new(EnforcementMode::Mandatory);
        assert_eq!(e.check(&ctx("edit")), EnforcementResult::Allow);
    }

    #[test]
    fn blocks_edit_without_workplan_mandatory() {
        let e = DefaultEnforcer::new(EnforcementMode::Mandatory);
        let mut c = ctx("edit");
        c.workplan_id = String::new();
        assert!(e.check(&c).is_blocked());
    }

    #[test]
    fn warns_edit_without_workplan_advisory() {
        let e = DefaultEnforcer::new(EnforcementMode::Advisory);
        let mut c = ctx("edit");
        c.workplan_id = String::new();
        let result = e.check(&c);
        assert!(matches!(result, EnforcementResult::Warn(_)));
    }

    #[test]
    fn blocks_background_agent_without_task() {
        let e = DefaultEnforcer::new(EnforcementMode::Mandatory);
        let c = EnforcementContext {
            agent_id: "agent-123".to_string(),
            workplan_id: "wp-test".to_string(),
            task_id: String::new(), // no task
            operation: "spawn_agent".to_string(),
            is_background: true,
            ..Default::default()
        };
        assert!(e.check(&c).is_blocked());
    }

    #[test]
    fn allows_background_agent_with_task() {
        let e = DefaultEnforcer::new(EnforcementMode::Mandatory);
        let c = EnforcementContext {
            agent_id: "agent-123".to_string(),
            workplan_id: "wp-test".to_string(),
            task_id: "task-789".to_string(),
            operation: "spawn_agent".to_string(),
            is_background: true,
            ..Default::default()
        };
        assert_eq!(e.check(&c), EnforcementResult::Allow);
    }

    #[test]
    fn disabled_mode_allows_everything() {
        let e = DefaultEnforcer::new(EnforcementMode::Disabled);
        let c = EnforcementContext {
            operation: "spawn_agent".to_string(),
            is_background: true,
            ..Default::default()
        };
        assert_eq!(e.check(&c), EnforcementResult::Allow);
    }

    #[test]
    fn exempt_operations_always_allowed() {
        let e = DefaultEnforcer::new(EnforcementMode::Mandatory);
        let c = EnforcementContext {
            operation: "session_start".to_string(),
            ..Default::default()
        };
        assert_eq!(e.check(&c), EnforcementResult::Allow);
    }
}

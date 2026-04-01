pub mod agent_manager;
pub mod constraint_enforcer;
pub mod context_pressure;
pub mod skill_selector;
pub mod workplan_executor;

/// Returns true when this process is running inside an active Claude Code session.
/// Claude Code sets CLAUDECODE=1 and CLAUDE_CODE_ENTRYPOINT in all child processes.
/// Used by the workplan executor and agent_manager to select Path B (inference queue)
/// vs Path A (direct inference gateway) per ADR-2604010000.
pub fn is_claude_code_session() -> bool {
    std::env::var("CLAUDECODE").as_deref() == Ok("1")
        || std::env::var("CLAUDE_CODE_ENTRYPOINT").is_ok()
}

#[cfg(test)]
mod session_tests {
    use super::*;

    #[test]
    fn detects_claudecode_env() {
        std::env::set_var("CLAUDECODE", "1");
        assert!(is_claude_code_session());
        std::env::remove_var("CLAUDECODE");
    }

    #[test]
    fn detects_entrypoint_env() {
        std::env::remove_var("CLAUDECODE");
        std::env::set_var("CLAUDE_CODE_ENTRYPOINT", "cli");
        assert!(is_claude_code_session());
        std::env::remove_var("CLAUDE_CODE_ENTRYPOINT");
    }

    #[test]
    fn returns_false_with_no_env() {
        std::env::remove_var("CLAUDECODE");
        std::env::remove_var("CLAUDE_CODE_ENTRYPOINT");
        assert!(!is_claude_code_session());
    }
}

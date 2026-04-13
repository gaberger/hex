pub mod agent;
pub mod agent_manager;
pub mod constraint_enforcer;
pub mod context_pressure;
pub mod directive;
pub mod errors;
pub mod grammars;
pub mod scaffolding;
pub mod skill_selector;
pub mod workplan_executor;

/// Returns the role-specific system prompt preamble to inject at the top of an agent's
/// task prompt. The preamble is the opening identity sentence from each role's
/// `hex-cli/assets/context-templates/roles/<role>/system.md` template — the static
/// portion that does not require variable substitution.
///
/// Full template rendering (with {{project_name}}, {{task_description}}, etc.) happens
/// in hex-agent's PromptPort adapter. This function only provides the identity header
/// so spawned agents immediately know their role and core responsibilities.
///
/// # Supported roles
/// - `"hex-coder"` — TDD implementation within a single adapter boundary
/// - `"hex-planner"` — workplan decomposition into adapter-bounded steps
/// - `"hex-reviewer"` — boundary enforcement and quality verdict (blocking gate)
/// - `"hex-integrator"` / `"integrator"` — merges worktrees and runs integration tests
/// - Any unknown role falls back to a generic "You are a <role> agent." line.
pub fn build_role_preamble(role: &str) -> String {
    match role {
        "hex-coder" | "coder" => {
            "You are a hex-coder agent operating inside the hex AIOS framework. \
Your role is to implement production-quality code within a single adapter boundary, \
following hexagonal architecture rules and a strict TDD workflow.\n\n"
                .to_string()
        }
        "hex-planner" | "planner" => {
            "You are a hex-planner agent operating inside the hex AIOS framework. \
Your role is to decompose feature requirements into a structured workplan where each \
step is bounded to a single adapter layer and safe to execute in an isolated git worktree.\n\n"
                .to_string()
        }
        "hex-reviewer" | "reviewer" => {
            "You are a hex-reviewer agent operating inside the hex AIOS framework. \
Your role is to enforce hexagonal architecture boundaries, identify boundary violations, \
and produce a structured quality verdict before integration. \
You are a blocking gate — do not approve code that violates architecture or safety constraints.\n\n"
                .to_string()
        }
        "hex-integrator" | "integrator" => {
            "You are a hex-integrator agent operating inside the hex AIOS framework. \
Your role is to merge adapter worktrees in dependency order, resolve conflicts, \
and validate end-to-end behaviour across all integration boundaries.\n\n"
                .to_string()
        }
        other if !other.is_empty() => {
            format!("You are a {} agent.\n\n", other)
        }
        _ => String::new(),
    }
}

/// Returns true when this process is running inside an active Claude Code session.
/// Claude Code sets CLAUDECODE=1 and CLAUDE_CODE_ENTRYPOINT in all child processes.
/// Used by the workplan executor and agent_manager to select Path B (inference queue)
/// vs Path A (direct inference gateway) per ADR-2604010000.
pub fn is_claude_code_session() -> bool {
    std::env::var("CLAUDECODE").as_deref() == Ok("1")
        || std::env::var("CLAUDE_CODE_ENTRYPOINT").is_ok()
}

#[cfg(test)]
mod role_preamble_tests {
    use super::*;

    #[test]
    fn hex_coder_preamble_contains_tdd() {
        let p = build_role_preamble("hex-coder");
        assert!(p.contains("TDD"), "hex-coder preamble should mention TDD workflow");
        assert!(p.ends_with("\n\n"));
    }

    #[test]
    fn coder_alias_matches_hex_coder() {
        assert_eq!(build_role_preamble("coder"), build_role_preamble("hex-coder"));
    }

    #[test]
    fn hex_planner_preamble_contains_workplan() {
        let p = build_role_preamble("hex-planner");
        assert!(p.contains("workplan"));
        assert!(p.ends_with("\n\n"));
    }

    #[test]
    fn hex_reviewer_preamble_contains_blocking_gate() {
        let p = build_role_preamble("hex-reviewer");
        assert!(p.contains("blocking gate"));
        assert!(p.ends_with("\n\n"));
    }

    #[test]
    fn hex_integrator_preamble_contains_merge() {
        let p = build_role_preamble("hex-integrator");
        assert!(p.contains("merge"));
        assert!(p.ends_with("\n\n"));
    }

    #[test]
    fn unknown_role_falls_back_to_generic() {
        let p = build_role_preamble("some-custom-agent");
        assert_eq!(p, "You are a some-custom-agent agent.\n\n");
    }

    #[test]
    fn empty_role_returns_empty_string() {
        assert_eq!(build_role_preamble(""), "");
    }
}

#[cfg(test)]
mod session_tests {
    use super::*;
    use std::sync::Mutex;

    // Serialize all env-var tests to prevent races in parallel test execution.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn detects_claudecode_env() {
        let _g = ENV_LOCK.lock().unwrap();
        std::env::set_var("CLAUDECODE", "1");
        std::env::remove_var("CLAUDE_CODE_ENTRYPOINT");
        assert!(is_claude_code_session());
        std::env::remove_var("CLAUDECODE");
    }

    #[test]
    fn detects_entrypoint_env() {
        let _g = ENV_LOCK.lock().unwrap();
        std::env::remove_var("CLAUDECODE");
        std::env::set_var("CLAUDE_CODE_ENTRYPOINT", "cli");
        assert!(is_claude_code_session());
        std::env::remove_var("CLAUDE_CODE_ENTRYPOINT");
    }

    #[test]
    fn returns_false_with_no_env() {
        let _g = ENV_LOCK.lock().unwrap();
        std::env::remove_var("CLAUDECODE");
        std::env::remove_var("CLAUDE_CODE_ENTRYPOINT");
        assert!(!is_claude_code_session());
    }
}

pub mod agent;
pub mod agent_manager;
pub mod constraint_enforcer;
pub mod context_pressure;
pub mod directive;
pub mod errors;
pub mod grammars;
pub mod regression;
pub mod scaffolding;
pub mod skill_selector;
pub mod workplan_executor;

use crate::ports::state::IHexFloMemoryStatePort;

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

/// Maximum number of taste preferences injected into an agent preamble.
const MAX_TASTE_PREFERENCES: usize = 10;

/// Minimum confidence threshold for a taste preference to be included.
const MIN_TASTE_CONFIDENCE: f64 = 0.5;

/// Builds a "Developer Preferences" section from taste entries stored in HexFlo memory.
///
/// Taste preferences are stored with keys like `taste:universal:naming:snake_case`.
/// Each value is a JSON object with at least `category`, `description`, and `confidence`
/// fields.  Tombstoned entries (`"deleted": true`) and low-confidence entries (below
/// [`MIN_TASTE_CONFIDENCE`]) are filtered out.  Results are sorted by confidence
/// descending and capped at [`MAX_TASTE_PREFERENCES`].
///
/// Returns an empty string when no qualifying preferences are found or when the
/// memory query fails (taste injection is best-effort — it must never block agent
/// spawning).
pub async fn build_taste_section(
    memory: &dyn IHexFloMemoryStatePort,
) -> String {
    let entries = match memory.hexflo_memory_search("taste:").await {
        Ok(v) => v,
        Err(_) => return String::new(),
    };

    // Parse each value as JSON and extract qualifying preferences.
    let mut prefs: Vec<(f64, String, String)> = entries
        .into_iter()
        .filter_map(|(_key, value)| {
            let obj: serde_json::Value = serde_json::from_str(&value).ok()?;

            // Skip tombstoned entries.
            if obj.get("deleted").and_then(|v| v.as_bool()).unwrap_or(false) {
                return None;
            }

            let confidence = obj.get("confidence").and_then(|v| v.as_f64()).unwrap_or(0.0);
            if confidence < MIN_TASTE_CONFIDENCE {
                return None;
            }

            let category = obj
                .get("category")
                .and_then(|v| v.as_str())
                .unwrap_or("general")
                .to_string();
            let description = obj
                .get("description")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            if description.is_empty() {
                return None;
            }

            Some((confidence, category, description))
        })
        .collect();

    if prefs.is_empty() {
        return String::new();
    }

    // Sort by confidence descending, then cap.
    prefs.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    prefs.truncate(MAX_TASTE_PREFERENCES);

    let mut section = String::from(
        "## Developer Preferences\n\
         These preferences have been set by the project developer. \
         Follow them unless they conflict with explicit task instructions.\n",
    );
    for (_confidence, category, description) in &prefs {
        section.push_str(&format!("- [{}] {}\n", category, description));
    }
    section.push('\n');

    section
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
mod taste_tests {
    use super::*;
    use crate::ports::state::{IHexFloMemoryStatePort, StateError};
    use async_trait::async_trait;

    /// Stub memory port that returns pre-configured search results.
    struct StubMemory {
        entries: Vec<(String, String)>,
    }

    #[async_trait]
    impl IHexFloMemoryStatePort for StubMemory {
        async fn hexflo_memory_store(&self, _k: &str, _v: &str, _s: &str) -> Result<(), StateError> {
            Ok(())
        }
        async fn hexflo_memory_retrieve(&self, _k: &str) -> Result<Option<String>, StateError> {
            Ok(None)
        }
        async fn hexflo_memory_search(&self, _q: &str) -> Result<Vec<(String, String)>, StateError> {
            Ok(self.entries.clone())
        }
        async fn hexflo_memory_delete(&self, _k: &str) -> Result<(), StateError> {
            Ok(())
        }
    }

    /// Stub that always errors — verifies graceful degradation.
    struct FailingMemory;

    #[async_trait]
    impl IHexFloMemoryStatePort for FailingMemory {
        async fn hexflo_memory_store(&self, _k: &str, _v: &str, _s: &str) -> Result<(), StateError> {
            Err(StateError::Connection("offline".into()))
        }
        async fn hexflo_memory_retrieve(&self, _k: &str) -> Result<Option<String>, StateError> {
            Err(StateError::Connection("offline".into()))
        }
        async fn hexflo_memory_search(&self, _q: &str) -> Result<Vec<(String, String)>, StateError> {
            Err(StateError::Connection("offline".into()))
        }
        async fn hexflo_memory_delete(&self, _k: &str) -> Result<(), StateError> {
            Err(StateError::Connection("offline".into()))
        }
    }

    #[tokio::test]
    async fn returns_empty_on_no_entries() {
        let mem = StubMemory { entries: vec![] };
        assert_eq!(build_taste_section(&mem).await, "");
    }

    #[tokio::test]
    async fn returns_empty_on_memory_failure() {
        let mem = FailingMemory;
        assert_eq!(build_taste_section(&mem).await, "");
    }

    #[tokio::test]
    async fn filters_tombstoned_entries() {
        let mem = StubMemory {
            entries: vec![(
                "taste:naming".into(),
                r#"{"category":"naming","description":"Use snake_case","confidence":0.9,"deleted":true}"#.into(),
            )],
        };
        assert_eq!(build_taste_section(&mem).await, "");
    }

    #[tokio::test]
    async fn filters_low_confidence() {
        let mem = StubMemory {
            entries: vec![(
                "taste:naming".into(),
                r#"{"category":"naming","description":"Use snake_case","confidence":0.3}"#.into(),
            )],
        };
        assert_eq!(build_taste_section(&mem).await, "");
    }

    #[tokio::test]
    async fn includes_qualifying_preference() {
        let mem = StubMemory {
            entries: vec![(
                "taste:naming".into(),
                r#"{"category":"naming","description":"Prefer snake_case for Rust function names","confidence":0.8}"#.into(),
            )],
        };
        let section = build_taste_section(&mem).await;
        assert!(section.contains("## Developer Preferences"));
        assert!(section.contains("- [naming] Prefer snake_case for Rust function names"));
    }

    #[tokio::test]
    async fn sorts_by_confidence_descending() {
        let mem = StubMemory {
            entries: vec![
                (
                    "taste:style".into(),
                    r#"{"category":"style","description":"Low prio","confidence":0.6}"#.into(),
                ),
                (
                    "taste:naming".into(),
                    r#"{"category":"naming","description":"High prio","confidence":0.95}"#.into(),
                ),
            ],
        };
        let section = build_taste_section(&mem).await;
        let high_pos = section.find("High prio").unwrap();
        let low_pos = section.find("Low prio").unwrap();
        assert!(high_pos < low_pos, "Higher confidence should come first");
    }

    #[tokio::test]
    async fn caps_at_max_preferences() {
        let entries: Vec<(String, String)> = (0..15)
            .map(|i| {
                let conf = 0.5 + (i as f64) * 0.03;
                (
                    format!("taste:pref{i}"),
                    format!(
                        r#"{{"category":"cat{i}","description":"Pref {i}","confidence":{conf}}}"#
                    ),
                )
            })
            .collect();
        let mem = StubMemory { entries };
        let section = build_taste_section(&mem).await;
        let count = section.matches("\n- [").count();
        assert_eq!(count, MAX_TASTE_PREFERENCES);
    }

    #[tokio::test]
    async fn skips_empty_description() {
        let mem = StubMemory {
            entries: vec![(
                "taste:empty".into(),
                r#"{"category":"naming","description":"","confidence":0.9}"#.into(),
            )],
        };
        assert_eq!(build_taste_section(&mem).await, "");
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

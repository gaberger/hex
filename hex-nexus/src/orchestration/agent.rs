/// Role assignment and template loading for agent registration (P6.2, ADR-2603312100).
///
/// This module is intentionally independent of `hex-agent::domain::context`.
/// hex-nexus sits at the orchestration boundary: it parses role strings arriving
/// from workplan task definitions and composes a prompt prefix from filesystem
/// templates without importing the agent crate.
use std::path::{Path, PathBuf};

// ── Role enum ──────────────────────────────────────────

/// Agent roles recognised by hex-nexus for context engineering.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentRole {
    Coder,
    Planner,
    Reviewer,
    Integrator,
}

impl AgentRole {
    /// Parse a role string. Accepts both bare names ("coder", "planner") and
    /// prefixed variants ("hex-coder", "hex-planner") as written in workplan
    /// `agent` fields.
    pub fn from_str(s: &str) -> Option<Self> {
        match s.trim().to_lowercase().trim_start_matches("hex-") {
            "coder" => Some(AgentRole::Coder),
            "planner" => Some(AgentRole::Planner),
            "reviewer" => Some(AgentRole::Reviewer),
            "integrator" => Some(AgentRole::Integrator),
            _ => None,
        }
    }

    /// Canonical directory name used for template lookup under `roles/`.
    pub fn template_dir_name(&self) -> &'static str {
        match self {
            AgentRole::Coder => "hex-coder",
            AgentRole::Planner => "hex-planner",
            AgentRole::Reviewer => "hex-reviewer",
            AgentRole::Integrator => "hex-integrator",
        }
    }

    /// Label string suitable for prompt injection ("You are a hex-coder agent.").
    pub fn as_str(&self) -> &'static str {
        self.template_dir_name()
    }
}

// ── Template file ordering ─────────────────────────────

/// Ordered list of template file names to load for each role, relative to
/// `<templates_base>/roles/<role>/`. Files are joined in order with double
/// newlines as section separators.
fn role_template_files(role: AgentRole) -> &'static [&'static str] {
    match role {
        AgentRole::Coder => &["system.md", "tools.md"],
        AgentRole::Planner => &["system.md", "task-assignment.md"],
        AgentRole::Reviewer => &["system.md"],
        AgentRole::Integrator => &["system.md"],
    }
}

// ── Template loading ───────────────────────────────────

/// Load and join role-specific template files from
/// `<templates_base>/roles/<role>/`.
///
/// Missing files are silently skipped so that optional templates (e.g.
/// `hex-integrator/system.md` when only the coder templates ship) never
/// cause failures. Returns an empty string if no files are readable.
pub fn load_role_templates(role: AgentRole, templates_base: &Path) -> String {
    let role_dir = templates_base.join("roles").join(role.template_dir_name());
    let mut sections = Vec::new();

    for filename in role_template_files(role) {
        let path = role_dir.join(filename);
        match std::fs::read_to_string(&path) {
            Ok(content) if !content.trim().is_empty() => {
                sections.push(content.trim().to_string());
            }
            Ok(_) => {
                tracing::debug!(path = %path.display(), "Role template is empty, skipping");
            }
            Err(e) => {
                tracing::debug!(path = %path.display(), err = %e, "Role template not found, skipping");
            }
        }
    }

    sections.join("\n\n")
}

/// Attempt to locate the `context-templates` directory using three strategies
/// in priority order:
///
/// 1. `HEX_TEMPLATES_DIR` environment variable override (highest priority).
/// 2. Relative to the current executable — covers both installed layouts and
///    Cargo debug/release builds in the workspace.
/// 3. Relative to `CARGO_MANIFEST_DIR` — fallback for `cargo test` runs.
pub fn find_templates_dir() -> Option<PathBuf> {
    // 1. Env override
    if let Ok(dir) = std::env::var("HEX_TEMPLATES_DIR") {
        let p = PathBuf::from(dir);
        if p.exists() {
            return Some(p);
        }
    }

    // 2. Exe-relative paths
    if let Ok(exe) = std::env::current_exe() {
        let candidates: &[Option<PathBuf>] = &[
            // Cargo workspace: <root>/target/{debug,release}/hex-nexus
            //   → <root>/hex-cli/assets/context-templates
            exe.parent()
                .and_then(|p| p.parent())
                .and_then(|p| p.parent())
                .map(|r| r.join("hex-cli").join("assets").join("context-templates")),
            // Installed layout: <prefix>/bin/hex-nexus
            //   → <prefix>/hex-cli/assets/context-templates
            exe.parent()
                .and_then(|p| p.parent())
                .map(|r| r.join("hex-cli").join("assets").join("context-templates")),
        ];
        for candidate in candidates.iter().flatten() {
            if candidate.exists() {
                tracing::debug!(path = %candidate.display(), "Found context-templates dir");
                return Some(candidate.clone());
            }
        }
    }

    None
}

// ── Public API ─────────────────────────────────────────

/// Build the role-specific prompt prefix for an agent given a raw role string
/// (e.g. `"hex-coder"`, `"planner"`).
///
/// Returns an empty string when:
/// - the role string is unknown,
/// - the context-templates directory cannot be located, or
/// - no template files are present for the role.
///
/// This function never errors — missing templates degrade gracefully to a
/// plain prompt without a prefix.
pub fn build_role_prompt(role_str: &str) -> String {
    let Some(role) = AgentRole::from_str(role_str) else {
        tracing::debug!(role = %role_str, "Unknown agent role — no template prefix added");
        return String::new();
    };

    let Some(templates_dir) = find_templates_dir() else {
        tracing::debug!("context-templates dir not found — no template prefix added");
        return String::new();
    };

    let content = load_role_templates(role, &templates_dir);
    if content.is_empty() {
        tracing::debug!(role = %role_str, "No template content found for role");
        return String::new();
    }

    tracing::info!(role = %role_str, "Loaded role-specific prompt templates");
    content
}

// ── Tests ──────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_bare_role_names() {
        assert_eq!(AgentRole::from_str("coder"), Some(AgentRole::Coder));
        assert_eq!(AgentRole::from_str("planner"), Some(AgentRole::Planner));
        assert_eq!(AgentRole::from_str("reviewer"), Some(AgentRole::Reviewer));
        assert_eq!(AgentRole::from_str("integrator"), Some(AgentRole::Integrator));
    }

    #[test]
    fn parse_prefixed_role_names() {
        assert_eq!(AgentRole::from_str("hex-coder"), Some(AgentRole::Coder));
        assert_eq!(AgentRole::from_str("hex-planner"), Some(AgentRole::Planner));
        assert_eq!(AgentRole::from_str("hex-reviewer"), Some(AgentRole::Reviewer));
        assert_eq!(AgentRole::from_str("hex-integrator"), Some(AgentRole::Integrator));
    }

    #[test]
    fn parse_case_insensitive() {
        assert_eq!(AgentRole::from_str("CODER"), Some(AgentRole::Coder));
        assert_eq!(AgentRole::from_str("Hex-Planner"), Some(AgentRole::Planner));
    }

    #[test]
    fn parse_unknown_role_returns_none() {
        assert_eq!(AgentRole::from_str("unknown-agent"), None);
        assert_eq!(AgentRole::from_str(""), None);
        assert_eq!(AgentRole::from_str("hex-"), None);
    }

    #[test]
    fn template_dir_names_are_consistent() {
        assert_eq!(AgentRole::Coder.template_dir_name(), "hex-coder");
        assert_eq!(AgentRole::Planner.template_dir_name(), "hex-planner");
        assert_eq!(AgentRole::Reviewer.template_dir_name(), "hex-reviewer");
        assert_eq!(AgentRole::Integrator.template_dir_name(), "hex-integrator");
    }

    #[test]
    fn role_as_str_matches_dir_name() {
        for role in [AgentRole::Coder, AgentRole::Planner, AgentRole::Reviewer, AgentRole::Integrator] {
            assert_eq!(role.as_str(), role.template_dir_name());
        }
    }

    #[test]
    fn load_role_templates_missing_dir_returns_empty() {
        let result = load_role_templates(AgentRole::Coder, Path::new("/nonexistent/templates"));
        assert!(result.is_empty());
    }

    #[test]
    fn load_role_templates_from_workspace() {
        // Locate templates relative to the cargo workspace root.
        // This test is skipped gracefully when templates are absent (CI without assets).
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let templates_dir = manifest_dir
            .parent()
            .unwrap()
            .join("hex-cli")
            .join("assets")
            .join("context-templates");

        if !templates_dir.exists() {
            return;
        }

        let content = load_role_templates(AgentRole::Coder, &templates_dir);
        assert!(!content.is_empty(), "hex-coder templates should produce non-empty output");
    }

    #[test]
    fn load_role_templates_sections_joined_with_double_newline() {
        // Create a temp dir with two fake template files and verify join behaviour.
        let tmp = tempfile::tempdir().expect("tempdir");
        let role_dir = tmp.path().join("roles").join("hex-coder");
        std::fs::create_dir_all(&role_dir).unwrap();
        std::fs::write(role_dir.join("system.md"), "# System\nContent A").unwrap();
        std::fs::write(role_dir.join("tools.md"), "# Tools\nContent B").unwrap();

        let content = load_role_templates(AgentRole::Coder, tmp.path());
        assert!(content.contains("Content A"), "system section missing");
        assert!(content.contains("Content B"), "tools section missing");
        assert!(content.contains("\n\n"), "sections should be separated by double newline");
    }

    #[test]
    fn load_role_templates_empty_file_is_skipped() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let role_dir = tmp.path().join("roles").join("hex-coder");
        std::fs::create_dir_all(&role_dir).unwrap();
        std::fs::write(role_dir.join("system.md"), "   ").unwrap(); // whitespace only
        std::fs::write(role_dir.join("tools.md"), "Real content").unwrap();

        let content = load_role_templates(AgentRole::Coder, tmp.path());
        assert_eq!(content, "Real content");
    }

    #[test]
    fn build_role_prompt_unknown_role_returns_empty() {
        assert!(build_role_prompt("unknown-role").is_empty());
        assert!(build_role_prompt("").is_empty());
    }
}

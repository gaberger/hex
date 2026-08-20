//! Repo-path allowlist used by every tool that touches the filesystem
//! (wp-sop-agent-loop P1.2).
//!
//! Mirrors the allowlist in `twin_reviewer.rs:810` and `code_patch.rs:97`.
//! TODO(post-P7): collapse all three into a single source of truth once
//! the agent-loop path is the default and the legacy drafter retires.
//! Keeping them separate during the rollout so the existing SOP loop
//! that just landed today is not perturbed.

const ALLOWED_PREFIXES: &[&str] = &[
    "docs/",
    "src/",
    "tests/",
    "examples/",
    "scripts/",
    "hex-nexus/src/",
    "hex-nexus/tests/",
    "hex-cli/src/",
    "hex-cli/tests/",
    "hex-core/src/",
    "hex-core/tests/",
    "hex-agent/src/",
    "hex-agent/tests/",
    "hex-parser/src/",
    "hex-parser/tests/",
    "hex-analyzer/src/",
    "hex-analyzer/tests/",
    "hex-desktop/src/",
    "hex-desktop/tests/",
    "hex-nexus/assets/src/",
    "hex-cli/assets/",
    "spacetime-modules/",
];

/// Returns `Ok(())` if `path` is a repo-relative path under one of the
/// allowed prefixes. Returns `Err(reason)` otherwise — the reason string
/// is operator-readable and gets bubbled up as a `ToolError::PolicyDenied`.
///
/// Rejection causes (in order checked):
/// - empty path
/// - absolute path (starts with `/`)
/// - path-traversal (`..` anywhere as a component)
/// - prefix not on the allowlist
pub fn allowed_repo_path(path: &str) -> Result<(), String> {
    if path.is_empty() {
        return Err("empty path".to_string());
    }
    if path.starts_with('/') {
        return Err(format!("absolute path rejected: {}", path));
    }
    for component in path.split('/') {
        if component == ".." {
            return Err(format!("path traversal rejected: {}", path));
        }
    }
    if !ALLOWED_PREFIXES.iter().any(|p| path.starts_with(p))
        && !path.ends_with("/Cargo.toml")
        && path != "Cargo.toml"
    {
        return Err(format!("path outside allowed prefixes: {}", path));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allows_hex_cli_tests() {
        assert!(allowed_repo_path("hex-cli/tests/standalone_gate.rs").is_ok());
    }

    #[test]
    fn allows_top_level_docs_and_examples() {
        assert!(allowed_repo_path("docs/specs/foo.md").is_ok());
        assert!(allowed_repo_path("examples/standalone-pipeline-test/run.sh").is_ok());
    }

    #[test]
    fn allows_cargo_toml() {
        assert!(allowed_repo_path("Cargo.toml").is_ok());
        assert!(allowed_repo_path("hex-cli/Cargo.toml").is_ok());
    }

    #[test]
    fn rejects_absolute_path() {
        let err = allowed_repo_path("/etc/passwd").unwrap_err();
        assert!(err.contains("absolute path"));
    }

    #[test]
    fn rejects_traversal() {
        let err = allowed_repo_path("hex-cli/tests/../../../etc/passwd").unwrap_err();
        assert!(err.contains("path traversal"));
    }

    #[test]
    fn rejects_empty() {
        assert!(allowed_repo_path("").is_err());
    }

    #[test]
    fn rejects_unknown_prefix() {
        let err = allowed_repo_path("foo/bar.rs").unwrap_err();
        assert!(err.contains("outside allowed prefixes"));
    }
}

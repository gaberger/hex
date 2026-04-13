//! Regression detector (ADR-2604131500 P2 P1.1).
//! Correlates gate failures with recent agent commits to identify
//! which scope should have trust decayed.

use tokio::process::Command;

/// A report linking a gate failure to a specific agent commit and hex scope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegressionReport {
    /// Hex architecture scope (e.g. "adapters/secondary", "domain")
    pub scope: String,
    /// Agent that made the commit
    pub agent_id: String,
    /// The offending commit hash
    pub commit_hash: String,
    /// Files modified in that commit
    pub files_changed: Vec<String>,
    /// The gate command that failed
    pub failing_command: String,
}

/// Map a file path to its hex architecture scope.
pub fn map_file_to_scope(path: &str) -> &'static str {
    if path.starts_with("src/core/domain/") || path.starts_with("domain/") {
        "domain"
    } else if path.starts_with("src/core/ports/") || path.starts_with("ports/") {
        "ports"
    } else if path.starts_with("src/adapters/primary/") || path.starts_with("adapters/primary/") {
        "adapters/primary"
    } else if path.starts_with("src/adapters/secondary/")
        || path.starts_with("adapters/secondary/")
    {
        "adapters/secondary"
    } else if path.contains("Cargo.toml") || path.contains("package.json") || path.contains("deps")
    {
        "dependencies"
    } else if path.contains("deploy") || path.contains("Dockerfile") || path.contains(".github") {
        "deployment"
    } else {
        "adapters/secondary"
    }
}

/// Returns true when the author name or email matches known agent patterns.
fn is_agent_author(name: &str, email: &str) -> bool {
    let name_lower = name.to_lowercase();
    let email_lower = email.to_lowercase();
    name_lower.contains("hex-coder")
        || name_lower.contains("claude")
        || name_lower.contains("hex-agent")
        || email_lower.contains("noreply@anthropic.com")
}

/// Detect regressions by correlating gate failures with recent agent commits.
///
/// Scans `git log` in `project_dir` for commits made by agents within the
/// last `lookback_seconds`, then maps changed files to hex scopes and
/// builds a [`RegressionReport`] per unique scope.
pub async fn detect_regression(
    project_dir: &str,
    failing_command: &str,
    lookback_seconds: u64,
) -> Vec<RegressionReport> {
    let since_arg = format!("--since={}s", lookback_seconds);
    let log_output = match Command::new("git")
        .args(["log", &since_arg, "--format=%H|%an|%ae", "--no-merges"])
        .current_dir(project_dir)
        .output()
        .await
    {
        Ok(o) => o,
        Err(_) => return Vec::new(),
    };

    if !log_output.status.success() {
        return Vec::new();
    }

    let stdout = String::from_utf8_lossy(&log_output.stdout);
    let mut reports: Vec<RegressionReport> = Vec::new();
    // Track scopes we already reported to deduplicate.
    let mut seen_scopes = std::collections::HashSet::new();

    for line in stdout.lines() {
        let parts: Vec<&str> = line.splitn(3, '|').collect();
        if parts.len() < 3 {
            continue;
        }
        let (hash, name, email) = (parts[0], parts[1], parts[2]);

        if !is_agent_author(name, email) {
            continue;
        }

        // Get changed files for this commit.
        let diff_arg = format!("{}^..{}", hash, hash);
        let diff_output = match Command::new("git")
            .args(["diff", "--name-only", &diff_arg])
            .current_dir(project_dir)
            .output()
            .await
        {
            Ok(o) if o.status.success() => o,
            _ => continue,
        };

        let diff_stdout = String::from_utf8_lossy(&diff_output.stdout);
        let files: Vec<String> = diff_stdout
            .lines()
            .filter(|l| !l.is_empty())
            .map(|l| l.to_string())
            .collect();

        // Group by scope.
        let mut scope_files: std::collections::HashMap<&str, Vec<String>> =
            std::collections::HashMap::new();
        for f in &files {
            let scope = map_file_to_scope(f);
            scope_files.entry(scope).or_default().push(f.clone());
        }

        for (scope, changed) in scope_files {
            if seen_scopes.insert(scope.to_string()) {
                reports.push(RegressionReport {
                    scope: scope.to_string(),
                    agent_id: name.to_string(),
                    commit_hash: hash.to_string(),
                    files_changed: changed,
                    failing_command: failing_command.to_string(),
                });
            }
        }
    }

    reports
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_map_file_to_scope() {
        assert_eq!(map_file_to_scope("src/core/domain/value-objects.ts"), "domain");
        assert_eq!(map_file_to_scope("domain/mod.rs"), "domain");
        assert_eq!(map_file_to_scope("src/core/ports/inference.ts"), "ports");
        assert_eq!(map_file_to_scope("ports/mod.rs"), "ports");
        assert_eq!(
            map_file_to_scope("src/adapters/primary/cli.ts"),
            "adapters/primary"
        );
        assert_eq!(
            map_file_to_scope("adapters/primary/http.rs"),
            "adapters/primary"
        );
        assert_eq!(
            map_file_to_scope("src/adapters/secondary/fs.ts"),
            "adapters/secondary"
        );
        assert_eq!(
            map_file_to_scope("adapters/secondary/sqlite.rs"),
            "adapters/secondary"
        );
        assert_eq!(map_file_to_scope("Cargo.toml"), "dependencies");
        assert_eq!(map_file_to_scope("some/package.json"), "dependencies");
        assert_eq!(map_file_to_scope("deps/openssl"), "dependencies");
        assert_eq!(map_file_to_scope("Dockerfile"), "deployment");
        assert_eq!(map_file_to_scope(".github/workflows/ci.yml"), "deployment");
        assert_eq!(map_file_to_scope("deploy/k8s.yml"), "deployment");
        // fallback
        assert_eq!(map_file_to_scope("src/main.rs"), "adapters/secondary");
        assert_eq!(map_file_to_scope("README.md"), "adapters/secondary");
    }

    #[test]
    fn test_agent_author_matching() {
        assert!(is_agent_author("hex-coder-1", "agent@example.com"));
        assert!(is_agent_author("Claude", "noreply@anthropic.com"));
        assert!(is_agent_author("hex-agent", "bot@ci.local"));
        assert!(is_agent_author("Some Name", "noreply@anthropic.com"));
        assert!(!is_agent_author("Gary Berger", "gary@example.com"));
        assert!(!is_agent_author("Developer", "dev@corp.com"));
    }

    #[tokio::test]
    async fn test_empty_git_log_returns_empty_reports() {
        // Use a non-existent directory so git fails gracefully.
        let reports = detect_regression("/tmp/nonexistent-repo-12345", "cargo test", 3600).await;
        assert!(reports.is_empty());
    }
}

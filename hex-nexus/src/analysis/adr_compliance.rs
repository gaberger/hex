//! ADR Compliance Checker (ADR-045).
//!
//! Scans source files for patterns that violate accepted Architecture Decision Records.
//! Rules are loaded from the **project's** `.hex/adr-rules.toml` — hex ships zero
//! project-specific rules. The engine is the framework; the rules are the project's.

use serde::{Deserialize, Serialize};
use std::path::Path;

// ── Types ──────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AdrViolation {
    pub adr: String,        // e.g. "ADR-039"
    pub rule_id: String,    // e.g. "adr-039-no-rest-state"
    pub file: String,
    pub line: usize,
    pub message: String,
    pub severity: AdrSeverity,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AdrSeverity {
    Error,
    Warning,
    Info,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AdrComplianceResult {
    pub violations: Vec<AdrViolation>,
    pub rules_checked: usize,
    pub files_scanned: usize,
    pub rules_file: Option<String>,
}

// ── Project-Level Rule Definition ──────────────────────
// Loaded from .hex/adr-rules.toml in the project root.

/// The TOML structure of `.hex/adr-rules.toml`:
/// ```toml
/// [[rules]]
/// adr = "ADR-039"
/// id = "adr-039-no-rest-state"
/// message = "REST handlers must not read from in-memory HashMap"
/// severity = "warning"
/// file_patterns = [".rs"]
/// exclude_patterns = ["state.rs", "test"]
/// violation_patterns = ["state.projects.read().await"]
/// ```
#[derive(Debug, Clone, Deserialize)]
struct AdrRulesFile {
    #[serde(default)]
    rules: Vec<AdrRuleConfig>,
}

#[derive(Debug, Clone, Deserialize)]
struct AdrRuleConfig {
    adr: String,
    id: String,
    message: String,
    #[serde(default = "default_severity")]
    severity: String,
    #[serde(default)]
    file_patterns: Vec<String>,
    #[serde(default)]
    exclude_patterns: Vec<String>,
    #[serde(default)]
    violation_patterns: Vec<String>,
}

fn default_severity() -> String {
    "warning".to_string()
}

/// Load rules from the project's `.hex/adr-rules.toml`.
fn load_rules(root_path: &Path) -> (Vec<AdrRuleConfig>, Option<String>) {
    let rules_path = root_path.join(".hex").join("adr-rules.toml");
    if !rules_path.is_file() {
        return (Vec::new(), None);
    }

    let path_str = rules_path.to_string_lossy().to_string();
    match std::fs::read_to_string(&rules_path) {
        Ok(content) => match toml::from_str::<AdrRulesFile>(&content) {
            Ok(parsed) => (parsed.rules, Some(path_str)),
            Err(e) => {
                tracing::warn!("Failed to parse {}: {}", path_str, e);
                (Vec::new(), Some(path_str))
            }
        },
        Err(e) => {
            tracing::warn!("Failed to read {}: {}", path_str, e);
            (Vec::new(), Some(path_str))
        }
    }
}

// ── Scanner ────────────────────────────────────────────

/// Check ADR compliance for all source files under `root_path`.
/// Rules are loaded from the project's `.hex/adr-rules.toml`.
pub async fn check_compliance(root_path: &Path) -> AdrComplianceResult {
    let (rules, rules_file) = load_rules(root_path);

    if rules.is_empty() {
        return AdrComplianceResult {
            violations: Vec::new(),
            rules_checked: 0,
            files_scanned: 0,
            rules_file,
        };
    }

    // Filter to rules with violation patterns
    let active_rules: Vec<&AdrRuleConfig> = rules
        .iter()
        .filter(|r| !r.violation_patterns.is_empty())
        .collect();

    let mut violations = Vec::new();
    let mut files_scanned = 0usize;

    let files = collect_files(root_path).await;

    for file_path in &files {
        let rel_path = file_path
            .strip_prefix(root_path)
            .unwrap_or(file_path)
            .to_string_lossy()
            .to_string();

        let content = match tokio::fs::read_to_string(file_path).await {
            Ok(c) => c,
            Err(_) => continue,
        };

        files_scanned += 1;

        for rule in &active_rules {
            // Check file pattern match
            if !rule.file_patterns.is_empty()
                && !rule.file_patterns.iter().any(|p| rel_path.ends_with(p.as_str()))
            {
                continue;
            }

            // Check exclusions
            if rule
                .exclude_patterns
                .iter()
                .any(|p| rel_path.contains(p.as_str()))
            {
                continue;
            }

            // Scan lines
            for (line_num, line) in content.lines().enumerate() {
                let trimmed = line.trim();
                if trimmed.starts_with("//")
                    || trimmed.starts_with("/*")
                    || trimmed.starts_with('*')
                    || trimmed.starts_with('#')
                {
                    continue;
                }

                for pattern in &rule.violation_patterns {
                    if line.contains(pattern.as_str()) {
                        let severity = match rule.severity.as_str() {
                            "error" => AdrSeverity::Error,
                            "info" => AdrSeverity::Info,
                            _ => AdrSeverity::Warning,
                        };

                        violations.push(AdrViolation {
                            adr: rule.adr.clone(),
                            rule_id: rule.id.clone(),
                            file: rel_path.clone(),
                            line: line_num + 1,
                            message: rule.message.clone(),
                            severity,
                        });
                        break;
                    }
                }
            }
        }
    }

    AdrComplianceResult {
        violations,
        rules_checked: active_rules.len(),
        files_scanned,
        rules_file,
    }
}

/// Synchronous version for the CLI (no async runtime needed).
pub fn check_compliance_sync(root_path: &Path) -> AdrComplianceResult {
    let (rules, rules_file) = load_rules(root_path);

    if rules.is_empty() {
        return AdrComplianceResult {
            violations: Vec::new(),
            rules_checked: 0,
            files_scanned: 0,
            rules_file,
        };
    }

    let active_rules: Vec<&AdrRuleConfig> = rules
        .iter()
        .filter(|r| !r.violation_patterns.is_empty())
        .collect();

    let mut violations = Vec::new();
    let mut files_scanned = 0usize;

    let files = collect_files_sync(root_path);

    for file_path in &files {
        let rel_path = file_path
            .strip_prefix(root_path)
            .unwrap_or(file_path)
            .to_string_lossy()
            .to_string();

        let content = match std::fs::read_to_string(file_path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        files_scanned += 1;

        for rule in &active_rules {
            if !rule.file_patterns.is_empty()
                && !rule.file_patterns.iter().any(|p| rel_path.ends_with(p.as_str()))
            {
                continue;
            }
            if rule.exclude_patterns.iter().any(|p| rel_path.contains(p.as_str())) {
                continue;
            }

            for (line_num, line) in content.lines().enumerate() {
                let trimmed = line.trim();
                if trimmed.starts_with("//") || trimmed.starts_with("/*")
                    || trimmed.starts_with('*') || trimmed.starts_with('#')
                {
                    continue;
                }

                for pattern in &rule.violation_patterns {
                    if line.contains(pattern.as_str()) {
                        let severity = match rule.severity.as_str() {
                            "error" => AdrSeverity::Error,
                            "info" => AdrSeverity::Info,
                            _ => AdrSeverity::Warning,
                        };
                        violations.push(AdrViolation {
                            adr: rule.adr.clone(),
                            rule_id: rule.id.clone(),
                            file: rel_path.clone(),
                            line: line_num + 1,
                            message: rule.message.clone(),
                            severity,
                        });
                        break;
                    }
                }
            }
        }
    }

    AdrComplianceResult {
        violations,
        rules_checked: active_rules.len(),
        files_scanned,
        rules_file,
    }
}

// ── File Collection ────────────────────────────────────

const SKIP_DIRS: &[&str] = &["node_modules", "target", "dist", ".git", "__pycache__", ".next"];

async fn collect_files(root: &Path) -> Vec<std::path::PathBuf> {
    let mut files = Vec::new();
    let mut stack = vec![root.to_path_buf()];

    while let Some(dir) = stack.pop() {
        let mut entries = match tokio::fs::read_dir(&dir).await {
            Ok(e) => e,
            Err(_) => continue,
        };
        while let Ok(Some(entry)) = entries.next_entry().await {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();

            if path.is_dir() {
                if !SKIP_DIRS.contains(&name.as_str()) {
                    stack.push(path);
                }
            } else if is_source_file(&name) {
                files.push(path);
            }
        }
    }
    files
}

fn collect_files_sync(root: &Path) -> Vec<std::path::PathBuf> {
    let mut files = Vec::new();
    let mut stack = vec![root.to_path_buf()];

    while let Some(dir) = stack.pop() {
        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();

            if path.is_dir() {
                if !SKIP_DIRS.contains(&name.as_str()) {
                    stack.push(path);
                }
            } else if is_source_file(&name) {
                files.push(path);
            }
        }
    }
    files
}

fn is_source_file(name: &str) -> bool {
    name.ends_with(".rs")
        || name.ends_with(".ts")
        || name.ends_with(".tsx")
        || name.ends_with(".js")
        || name.ends_with(".go")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_rules_returns_clean_result() {
        let result = check_compliance_sync(Path::new("/nonexistent"));
        assert_eq!(result.rules_checked, 0);
        assert!(result.violations.is_empty());
    }

    #[test]
    fn parses_rule_config() {
        let toml_str = r#"
[[rules]]
adr = "ADR-001"
id = "test-rule"
message = "Test violation"
severity = "error"
file_patterns = [".rs"]
violation_patterns = ["bad_pattern"]
"#;
        let parsed: AdrRulesFile = toml::from_str(toml_str).unwrap();
        assert_eq!(parsed.rules.len(), 1);
        assert_eq!(parsed.rules[0].adr, "ADR-001");
        assert_eq!(parsed.rules[0].violation_patterns, vec!["bad_pattern"]);
    }
}

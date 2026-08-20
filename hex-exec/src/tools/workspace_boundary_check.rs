//! `workspace_boundary_check` — enforces workspace-level hexagonal boundaries.
//!
//! Implements ADR-2026-05-09-0000 rules: validates that workspace crates only depend
//! on allowed peers. Scans Cargo.toml [dependencies] for forbidden workspace
//! deps, then walks src/**/*.rs for `use hex_*::` statements that violate the
//! rule table. Returns violations[] array for CI/pre-commit integration.

use async_trait::async_trait;
use regex::Regex;
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;
use walkdir::WalkDir;

use super::{Tool, ToolResult};

/// Canonical workspace boundary rules from ADR-2026-05-09-0000.
/// Format: (crate_name, &[allowed_dependencies])
const RULE_TABLE: &[(&str, &[&str])] = &[
    ("hex-core", &[]),
    ("hex-cli", &["hex-core"]),
    ("hex-nexus", &["hex-core", "hex-parser", "hex-analyzer"]),
    ("hex-analyzer", &["hex-core"]),
    ("hex-agent", &["hex-core", "hex-nexus"]),
    ("hex-parser", &[]),
    ("hex-desktop", &["hex-core"]),
];

#[derive(Debug, Serialize)]
struct BoundaryCheckResult {
    violations: Vec<BoundaryViolation>,
    total_violations: u64,
    crates_scanned: Vec<String>,
    scanned_files: usize,
}

#[derive(Debug, Serialize)]
struct BoundaryViolation {
    kind: ViolationKind,
    from_crate: String,
    to_crate: String,
    file: Option<String>,
    line: Option<usize>,
    statement: Option<String>,
    rule: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
enum ViolationKind {
    CargoToml,
    SourceImport,
}

pub struct WorkspaceBoundaryCheck;

#[async_trait]
impl Tool for WorkspaceBoundaryCheck {
    fn name(&self) -> &'static str {
        "workspace_boundary_check"
    }
    fn description(&self) -> &'static str {
        "Enforce workspace-level hexagonal boundaries per ADR-2026-05-09-0000. \
         Validates cross-crate dependencies against rule table, scans \
         Cargo.toml [dependencies] and src/**/*.rs for violations. Returns \
         violations array for CI/pre-commit integration."
    }
    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "fail_on_violations": {
                    "type": "boolean",
                    "description": "If true (default), tool returns error when violations found. If false, returns ok with violations in output.",
                },
                "verbose": {
                    "type": "boolean",
                    "description": "If true, include all scanned files in output. Default false.",
                }
            },
            "required": []
        })
    }

    async fn execute(&self, input: Value) -> ToolResult {
        let start = Instant::now();
        let fail_on_violations = input
            .get("fail_on_violations")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);
        let verbose = input.get("verbose").and_then(|v| v.as_bool()).unwrap_or(false);

        let repo_root = std::env::var("HEX_REPO_ROOT")
            .unwrap_or_else(|_| "/home/gary/hex-intf".to_string());
        let repo_path = Path::new(&repo_root);

        // Build rule map for O(1) lookups
        let rule_map: HashMap<&str, HashSet<&str>> = RULE_TABLE
            .iter()
            .map(|(crate_name, allowed)| (*crate_name, allowed.iter().copied().collect()))
            .collect();

        let mut violations = Vec::new();
        let mut crates_scanned = Vec::new();
        let mut scanned_files = 0;

        // Discover workspace crates
        let workspace_crates = match discover_workspace_crates(repo_path) {
            Ok(crates) => crates,
            Err(e) => {
                return ToolResult::err(
                    format!("Failed to discover workspace crates: {}", e),
                    start.elapsed().as_millis() as u64,
                );
            }
        };

        // Check each crate
        for (crate_name, crate_path) in workspace_crates {
            crates_scanned.push(crate_name.clone());

            // 1. Check Cargo.toml dependencies
            let cargo_toml_path = crate_path.join("Cargo.toml");
            if let Ok(deps) = parse_workspace_dependencies(&cargo_toml_path) {
                let allowed = rule_map.get(crate_name.as_str()).cloned().unwrap_or_default();
                for dep in deps {
                    if !allowed.contains(dep.as_str()) {
                        violations.push(BoundaryViolation {
                            kind: ViolationKind::CargoToml,
                            from_crate: crate_name.clone(),
                            to_crate: dep.clone(),
                            file: Some(cargo_toml_path.display().to_string()),
                            line: None,
                            statement: None,
                            rule: format!("{} → {} FORBIDDEN (allowed: [{}])", 
                                crate_name, dep, allowed.iter().copied().collect::<Vec<_>>().join(", ")),
                        });
                    }
                }
            }

            // 2. Scan source files for use statements
            let src_path = crate_path.join("src");
            if src_path.exists() {
                let use_pattern = Regex::new(r"^\s*use\s+(hex_[a-z_]+)::").unwrap();
                for entry in WalkDir::new(&src_path)
                    .follow_links(false)
                    .into_iter()
                    .filter_map(|e| e.ok())
                    .filter(|e| e.path().extension().and_then(|s| s.to_str()) == Some("rs"))
                {
                    scanned_files += 1;
                    let file_path = entry.path();
                    if let Ok(content) = fs::read_to_string(file_path) {
                        for (line_num, line) in content.lines().enumerate() {
                            if let Some(caps) = use_pattern.captures(line) {
                                let imported_module = &caps[1];
                                // Convert hex_nexus → hex-nexus
                                let imported_crate = imported_module.replace('_', "-");
                                
                                // Check if this is a workspace crate and if it's allowed
                                if rule_map.contains_key(imported_crate.as_str()) {
                                    let allowed = rule_map.get(crate_name.as_str()).cloned().unwrap_or_default();
                                    if !allowed.contains(imported_crate.as_str()) {
                                        violations.push(BoundaryViolation {
                                            kind: ViolationKind::SourceImport,
                                            from_crate: crate_name.clone(),
                                            to_crate: imported_crate.clone(),
                                            file: Some(file_path.display().to_string()),
                                            line: Some(line_num + 1),
                                            statement: Some(line.trim().chars().take(80).collect()),
                                            rule: format!("{} → {} FORBIDDEN (allowed: [{}])", 
                                                crate_name, imported_crate, 
                                                allowed.iter().copied().collect::<Vec<_>>().join(", ")),
                                        });
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        let total_violations = violations.len() as u64;
        let elapsed = start.elapsed().as_millis() as u64;

        let result = BoundaryCheckResult {
            violations,
            total_violations,
            crates_scanned,
            scanned_files,
        };

        let output = if verbose {
            serde_json::to_value(&result).unwrap_or(json!(result))
        } else {
            json!({
                "violations": result.violations,
                "total_violations": result.total_violations,
                "crates_scanned": result.crates_scanned,
            })
        };

        if fail_on_violations && total_violations > 0 {
            ToolResult::err(
                format!(
                    "Found {} workspace boundary violation(s). See output for details.",
                    total_violations
                ),
                elapsed,
            )
        } else {
            ToolResult::ok(output, elapsed)
        }
    }
}

/// Discover workspace member crates by parsing workspace Cargo.toml.
fn discover_workspace_crates(repo_root: &Path) -> Result<Vec<(String, PathBuf)>, String> {
    let workspace_toml = repo_root.join("Cargo.toml");
    let content = fs::read_to_string(&workspace_toml).map_err(|e| e.to_string())?;
    let toml: toml::Value = toml::from_str(&content).map_err(|e| e.to_string())?;

    let mut crates = Vec::new();
    if let Some(workspace) = toml.get("workspace") {
        if let Some(members) = workspace.get("members").and_then(|v| v.as_array()) {
            for member in members {
                if let Some(member_path) = member.as_str() {
                    // Extract crate name from member path (e.g., "hex-cli" from "hex-cli" or "crates/hex-cli")
                    let crate_name = member_path.split('/').last().unwrap_or(member_path);
                    let full_path = repo_root.join(member_path);
                    if full_path.exists() {
                        crates.push((crate_name.to_string(), full_path));
                    }
                }
            }
        }
    }
    Ok(crates)
}

/// Parse workspace dependencies from a crate's Cargo.toml.
/// Returns only dependencies with `path = "../<crate>"` (workspace-internal).
fn parse_workspace_dependencies(cargo_toml: &Path) -> Result<Vec<String>, String> {
    let content = fs::read_to_string(cargo_toml).map_err(|e| e.to_string())?;
    let toml: toml::Value = toml::from_str(&content).map_err(|e| e.to_string())?;

    let mut deps = Vec::new();
    if let Some(dependencies) = toml.get("dependencies").and_then(|v| v.as_table()) {
        for (_dep_name, dep_spec) in dependencies {
            // Check if this is a workspace dependency (has `path = "../<crate>"`)
            if let Some(table) = dep_spec.as_table() {
                if let Some(path) = table.get("path").and_then(|v| v.as_str()) {
                    // Extract crate name from path
                    if path.starts_with("../") || path.starts_with("./") {
                        let crate_name = path.trim_start_matches("../").trim_start_matches("./");
                        deps.push(crate_name.to_string());
                    }
                }
            }
        }
    }
    Ok(deps)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rule_table_has_core_crates() {
        let map: HashMap<&str, HashSet<&str>> = RULE_TABLE
            .iter()
            .map(|(k, v)| (*k, v.iter().copied().collect()))
            .collect();
        assert_eq!(map.get("hex-core"), Some(&HashSet::new()));
        assert!(map.get("hex-cli").unwrap().contains("hex-core"));
        assert!(map.get("hex-nexus").unwrap().contains("hex-core"));
    }

    #[test]
    fn schema_is_object() {
        let tool = WorkspaceBoundaryCheck;
        let schema = tool.input_schema();
        assert_eq!(schema.get("type").and_then(|v| v.as_str()), Some("object"));
    }
}

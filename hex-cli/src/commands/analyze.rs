//! Architecture health check command.
//!
//! `hex analyze [path]` — checks hex layer structure using `hex_core::rules::boundary`
//! types, and when nexus is running, delegates to the full tree-sitter boundary analysis.

use std::path::{Path, PathBuf};

use colored::Colorize;
use hex_core::rules::boundary::{self, Layer};

use crate::nexus_client::NexusClient;

/// Layer directories to check, paired with their expected `hex_core` Layer enum.
const LAYER_DIRS: &[(&str, &str, Layer)] = &[
    ("core/domain", "Domain", Layer::Domain),
    ("core/ports", "Ports", Layer::Ports),
    ("core/usecases", "Use Cases", Layer::Usecases),
    ("adapters/primary", "Primary Adapters", Layer::AdapterPrimary),
    ("adapters/secondary", "Secondary Adapters", Layer::AdapterSecondary),
];

pub async fn run(path: &str, strict: bool, adr_compliance_only: bool, json_output: bool) -> anyhow::Result<()> {
    let root = Path::new(path)
        .canonicalize()
        .unwrap_or_else(|_| Path::new(path).to_path_buf());

    // JSON mode: collect results and emit structured output
    if json_output {
        return run_json(&root, strict, adr_compliance_only).await;
    }

    println!(
        "{} Architecture analysis: {}",
        "\u{2b21}".cyan(),
        root.display()
    );
    println!();

    // If --adr-compliance flag is set, skip boundary analysis entirely
    if !adr_compliance_only {
        // Check for hex project markers
        let has_src = root.join("src").is_dir();
        let has_package_json = root.join("package.json").is_file();
        let has_cargo_toml = root.join("Cargo.toml").is_file();
        let has_hex_config = root.join(".hex").is_dir();
        let has_docs_adrs = root.join("docs").join("adrs").is_dir();

        println!("  {}", "Project structure:".bold());
        print_check("src/ directory", has_src);
        print_check("package.json", has_package_json);
        print_check("Cargo.toml", has_cargo_toml);
        print_check(".hex/ config", has_hex_config);
        print_check("docs/adrs/", has_docs_adrs);

        // Check hex architecture layers using hex_core boundary types
        if has_src {
            println!();
            println!("  {}", "Hex layers:".bold());

            for (dir, label, expected_layer) in LAYER_DIRS {
                let layer_path = root.join("src").join(dir);
                let exists = layer_path.is_dir();
                print_check(label, exists);

                // Verify boundary::detect_layer agrees with our expectation
                if exists {
                    let detected = boundary::detect_layer(&format!("src/{}/mod.rs", dir));
                    if detected != *expected_layer {
                        println!(
                            "      {} Layer mismatch: expected {}, detected {}",
                            "!".yellow(),
                            expected_layer,
                            detected,
                        );
                    }
                }
            }

            let has_composition_root = root.join("src").join("composition-root.ts").is_file()
                || root.join("src").join("composition_root.rs").is_file();
            print_check("Composition Root", has_composition_root);
        }

        // Offline boundary check: scan for obvious violations without nexus
        if has_src {
            let violations = scan_local_violations(&root);
            if !violations.is_empty() {
                println!();
                println!(
                    "  {} Local boundary violations ({})",
                    "\u{26a0}".yellow(),
                    violations.len()
                );
                for v in &violations {
                    println!(
                        "    {} {} -> {} ({})",
                        "\u{2717}".red(),
                        v.source_file,
                        v.imported_path,
                        v.rule,
                    );
                }
            }
        }

        // Try nexus for full boundary analysis
        println!();
        let nexus = NexusClient::from_env();
        if nexus.ensure_running().await.is_ok() {
            // Register/push project to get analysis
            let _project_name = root
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("unknown");

            // Query project health if registered
            match nexus.get("/api/projects").await {
                Ok(resp) => {
                    if let Some(projects) = resp.get("projects").and_then(|p| p.as_array()) {
                        let matching = projects.iter().find(|p| {
                            p["rootPath"]
                                .as_str()
                                .map(|rp| root.to_string_lossy().contains(rp) || rp.contains(&*root.to_string_lossy()))
                                .unwrap_or(false)
                        });

                        if let Some(project) = matching {
                            let pid = project["id"].as_str().unwrap_or("-");
                            println!(
                                "  {} Project registered in nexus: {}",
                                "\u{2713}".green(),
                                pid
                            );

                            // Get health data
                            let health_path = format!("/api/{}/health", pid);
                            if let Ok(health) = nexus.get(&health_path).await {
                                if let Some(score) = health["score"].as_u64() {
                                    let grade = match score {
                                        90..=100 => format!("A ({})", score).green().to_string(),
                                        80..=89 => format!("B ({})", score).yellow().to_string(),
                                        _ => format!("C ({})", score).red().to_string(),
                                    };
                                    println!("  Grade: {}", grade);
                                }
                                if let Some(violations) = health["violations"].as_u64() {
                                    let v = if violations == 0 {
                                        "0".green().to_string()
                                    } else {
                                        violations.to_string().red().to_string()
                                    };
                                    println!("  Boundary violations: {}", v);
                                }
                            }
                        } else {
                            println!(
                                "  {} Project not registered in nexus — start nexus for full analysis",
                                "\u{25cb}".dimmed()
                            );
                            println!(
                                "  {} hex nexus start",
                                "\u{2192}".dimmed()
                            );
                        }
                    }
                }
                Err(_) => {
                    println!(
                        "  {} Could not query nexus for full analysis",
                        "\u{25cb}".dimmed()
                    );
                }
            }
        } else {
            println!(
                "  {} Full boundary analysis requires hex-nexus",
                "\u{2192}".dimmed()
            );
            println!(
                "  {} Start with: hex nexus start",
                "\u{2192}".dimmed()
            );
        }
    }

    // ADR compliance check (ADR-045) — runs locally, no nexus needed
    println!();
    println!("  {}", "ADR compliance:".bold());
    let adr_violations = check_adr_compliance(&root);
    let error_count = adr_violations.iter().filter(|v| v.severity == "error").count();
    let warning_count = adr_violations.iter().filter(|v| v.severity == "warning").count();

    if adr_violations.is_empty() {
        println!(
            "    {} All ADR rules satisfied",
            "\u{2713}".green()
        );
    } else {
        println!(
            "    {} {} ADR violation(s): {} error(s), {} warning(s)",
            "\u{26a0}".yellow(),
            adr_violations.len(),
            error_count,
            warning_count,
        );
        for v in &adr_violations {
            let icon = if v.severity == "error" {
                "\u{2717}".red()
            } else {
                "\u{26a0}".yellow()
            };
            println!(
                "    {} [{}] {}:{} — {}",
                icon, v.adr, v.file, v.line, v.message,
            );
        }
    }

    // Store compliance results in HexFlo memory (best-effort)
    store_compliance_in_hexflo(&adr_violations, error_count, warning_count).await;

    // --strict: exit with code 1 if any violations exist (warnings promoted to errors)
    if strict && !adr_violations.is_empty() {
        println!();
        println!(
            "  {} --strict mode: {} violation(s) found — exiting with code 1",
            "\u{2717}".red(),
            adr_violations.len(),
        );
        std::process::exit(1);
    }

    Ok(())
}

/// Scan source files for boundary violations using `hex_core::rules::boundary`.
///
/// This performs a lightweight offline check by inspecting Rust `use` and
/// TypeScript `import` statements without needing tree-sitter or nexus.
fn scan_local_violations(root: &Path) -> Vec<boundary::Violation> {
    let src = root.join("src");
    let mut all_violations = Vec::new();

    let files = collect_source_files(&src);

    for path in &files {
        let rel = path
            .strip_prefix(root)
            .unwrap_or(path)
            .to_string_lossy()
            .to_string();

        let source_layer = boundary::detect_layer(&rel);
        if source_layer == Layer::Unknown || source_layer == Layer::CompositionRoot {
            continue;
        }

        // Read file and extract import-like paths (best-effort, not a full parser)
        if let Ok(contents) = std::fs::read_to_string(path) {
            let imports = extract_import_paths(&contents, &rel);
            let violations = boundary::validate_imports(&rel, &imports);
            all_violations.extend(violations);
        }
    }

    all_violations
}

/// Recursively collect `.rs`, `.ts`, and `.js` source files under a directory.
fn collect_source_files(dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    collect_source_files_recursive(dir, &mut files);
    files
}

fn collect_source_files_recursive(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_source_files_recursive(&path, out);
        } else if let Some(ext) = path.extension().and_then(|x| x.to_str()) {
            if matches!(ext, "rs" | "ts" | "js") {
                out.push(path);
            }
        }
    }
}

/// Extract import/use paths from source text, resolving relative paths against
/// the source file's directory so layer detection works correctly.
///
/// `source_rel` is the source file path relative to the project root,
/// e.g. `src/core/ports/app-context.ts`.
fn extract_import_paths(source: &str, source_rel: &str) -> Vec<String> {
    let mut paths = Vec::new();

    // Directory of the source file (e.g. "src/core/ports" for "src/core/ports/app-context.ts")
    let source_dir = Path::new(source_rel)
        .parent()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();

    for line in source.lines() {
        let trimmed = line.trim();
        // Rust: use crate::adapters::...
        if let Some(rest) = trimmed.strip_prefix("use crate::") {
            if let Some(path_part) = rest.split(';').next() {
                // Convert module path to a directory-like path for layer detection
                let as_path = format!("src/{}", path_part.split("::").collect::<Vec<_>>().join("/"));
                paths.push(as_path);
            }
        }
        // TypeScript: import ... from './adapters/...'  or  from '../adapters/...'
        if trimmed.contains("from") && (trimmed.contains("import") || trimmed.contains("export")) {
            if let Some(start) = trimmed.find('\'').or_else(|| trimmed.find('"')) {
                let rest = &trimmed[start + 1..];
                if let Some(end) = rest.find('\'').or_else(|| rest.find('"')) {
                    let import_path = &rest[..end];
                    if import_path.starts_with('.') {
                        // Resolve relative path against source file's directory
                        let resolved = resolve_relative_path(&source_dir, import_path);
                        paths.push(resolved);
                    }
                }
            }
        }
    }
    paths
}

/// Resolve a relative import path (e.g. `../../core/ports/swarm.js`)
/// against a source directory (e.g. `src/core/ports`).
///
/// Returns a normalized path like `src/core/ports/swarm.js`.
fn resolve_relative_path(source_dir: &str, import_path: &str) -> String {
    let mut parts: Vec<&str> = source_dir.split('/').filter(|s| !s.is_empty()).collect();

    for segment in import_path.split('/') {
        match segment {
            "." | "" => {} // current dir — skip
            ".." => { parts.pop(); } // go up one level
            other => parts.push(other),
        }
    }

    parts.join("/")
}

// ── ADR Compliance (ADR-045) ────────────────────────────
// Rules are loaded from the project's `.hex/adr-rules.toml` — hex ships no
// project-specific rules. The engine is the framework; the rules are the project's.

struct AdrViolationLocal {
    adr: String,
    file: String,
    line: usize,
    message: String,
    severity: String,
}

#[derive(serde::Deserialize)]
struct AdrRulesFile {
    #[serde(default)]
    rules: Vec<AdrRuleConfig>,
}

#[derive(serde::Deserialize)]
struct AdrRuleConfig {
    adr: String,
    #[allow(dead_code)]
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

fn default_severity() -> String { "warning".to_string() }

fn check_adr_compliance(root: &Path) -> Vec<AdrViolationLocal> {
    // Load rules from project's .hex/adr-rules.toml
    let rules_path = root.join(".hex").join("adr-rules.toml");
    let rules = if rules_path.is_file() {
        match std::fs::read_to_string(&rules_path) {
            Ok(content) => match toml::from_str::<AdrRulesFile>(&content) {
                Ok(parsed) => {
                    println!(
                        "    {} Loaded {} rule(s) from {}",
                        "\u{2713}".green(),
                        parsed.rules.len(),
                        rules_path.strip_prefix(root).unwrap_or(&rules_path).display(),
                    );
                    parsed.rules
                }
                Err(e) => {
                    println!(
                        "    {} Failed to parse adr-rules.toml: {}",
                        "\u{2717}".red(), e
                    );
                    return Vec::new();
                }
            },
            Err(_) => return Vec::new(),
        }
    } else {
        println!(
            "    {} No .hex/adr-rules.toml found — skipping compliance check",
            "\u{25cb}".dimmed()
        );
        return Vec::new();
    };

    let active_rules: Vec<&AdrRuleConfig> = rules
        .iter()
        .filter(|r| !r.violation_patterns.is_empty())
        .collect();

    let mut violations = Vec::new();
    let files = collect_source_files(&root.join("src"));

    // Also scan hex-nexus/src if it exists (multi-crate projects)
    let mut all_files = files;
    for subdir in &["hex-nexus/src", "hex-cli/src"] {
        let sub = root.join(subdir);
        if sub.is_dir() {
            all_files.extend(collect_source_files(&sub));
        }
    }

    for path in &all_files {
        let rel = path
            .strip_prefix(root)
            .unwrap_or(path)
            .to_string_lossy()
            .to_string();

        let content = match std::fs::read_to_string(path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        for rule in &active_rules {
            if !rule.file_patterns.is_empty()
                && !rule.file_patterns.iter().any(|p| rel.ends_with(p.as_str()))
            {
                continue;
            }
            if rule.exclude_patterns.iter().any(|p| rel.contains(p.as_str())) {
                continue;
            }

            for (line_num, line) in content.lines().enumerate() {
                let trimmed = line.trim();
                if trimmed.starts_with("//") || trimmed.starts_with("/*") || trimmed.starts_with('*') {
                    continue;
                }
                for pattern in &rule.violation_patterns {
                    if line.contains(pattern.as_str()) {
                        violations.push(AdrViolationLocal {
                            adr: rule.adr.to_string(),
                            file: rel.clone(),
                            line: line_num + 1,
                            message: rule.message.to_string(),
                            severity: rule.severity.clone(),
                        });
                        break;
                    }
                }
            }
        }
    }

    violations
}

/// Store ADR compliance results in HexFlo memory via nexus REST API.
/// Best-effort: silently skips if nexus is not running.
async fn store_compliance_in_hexflo(
    violations: &[AdrViolationLocal],
    error_count: usize,
    warning_count: usize,
) {
    let nexus = NexusClient::from_env();
    if nexus.ensure_running().await.is_err() {
        return; // nexus not running — skip silently
    }

    let violation_details: Vec<serde_json::Value> = violations
        .iter()
        .map(|v| {
            serde_json::json!({
                "adr": v.adr,
                "file": v.file,
                "line": v.line,
                "message": v.message,
                "severity": v.severity,
            })
        })
        .collect();

    let payload = serde_json::json!({
        "key": "adr-compliance:default",
        "value": serde_json::json!({
            "violationCount": violations.len(),
            "errorCount": error_count,
            "warningCount": warning_count,
            "violations": violation_details,
            "checkedAt": chrono::Utc::now().to_rfc3339(),
        }).to_string(),
    });

    // Best-effort POST — ignore errors
    let _ = nexus.post("/api/hexflo/memory", &payload).await;
}

/// JSON output mode for `hex analyze --json`.
async fn run_json(root: &Path, strict: bool, adr_compliance_only: bool) -> anyhow::Result<()> {
    let mut result = serde_json::json!({});

    if !adr_compliance_only {
        // Local boundary violations
        let has_src = root.join("src").is_dir();
        let violations: Vec<serde_json::Value> = if has_src {
            scan_local_violations(root)
                .iter()
                .map(|v| {
                    serde_json::json!({
                        "source_file": v.source_file,
                        "imported_path": v.imported_path,
                        "rule": v.rule,
                    })
                })
                .collect()
        } else {
            Vec::new()
        };

        // Try nexus for score
        let nexus = NexusClient::from_env();
        let mut score: Option<u64> = None;
        let mut boundary_errors: Vec<serde_json::Value> = Vec::new();
        if nexus.ensure_running().await.is_ok() {
            if let Ok(resp) = nexus.get("/api/projects").await {
                if let Some(projects) = resp.get("projects").and_then(|p| p.as_array()) {
                    let matching = projects.iter().find(|p| {
                        p["rootPath"]
                            .as_str()
                            .map(|rp| root.to_string_lossy().contains(rp) || rp.contains(&*root.to_string_lossy()))
                            .unwrap_or(false)
                    });
                    if let Some(project) = matching {
                        let pid = project["id"].as_str().unwrap_or("-");
                        let health_path = format!("/api/{}/health", pid);
                        if let Ok(health) = nexus.get(&health_path).await {
                            score = health["score"].as_u64();
                            if let Some(v) = health["violations"].as_u64() {
                                boundary_errors.push(serde_json::json!({"count": v}));
                            }
                        }
                    }
                }
            }
        }

        result["score"] = score.map(serde_json::Value::from).unwrap_or(serde_json::Value::Null);
        result["violations"] = serde_json::Value::Array(violations);
        result["boundary_errors"] = serde_json::Value::Array(boundary_errors);
    }

    // ADR compliance
    let adr_violations = check_adr_compliance(root);
    let error_count = adr_violations.iter().filter(|v| v.severity == "error").count();
    let warning_count = adr_violations.iter().filter(|v| v.severity == "warning").count();

    let adr_details: Vec<serde_json::Value> = adr_violations
        .iter()
        .map(|v| {
            serde_json::json!({
                "adr": v.adr,
                "file": v.file,
                "line": v.line,
                "message": v.message,
                "severity": v.severity,
            })
        })
        .collect();

    result["adr_compliance"] = serde_json::json!({
        "violation_count": adr_violations.len(),
        "error_count": error_count,
        "warning_count": warning_count,
        "violations": adr_details,
    });

    // Best-effort store in HexFlo
    store_compliance_in_hexflo(&adr_violations, error_count, warning_count).await;

    println!("{}", serde_json::to_string_pretty(&result)?);

    if strict && !adr_violations.is_empty() {
        std::process::exit(1);
    }

    Ok(())
}

fn print_check(label: &str, present: bool) {
    let indicator = if present {
        "\u{2713}".green()
    } else {
        "\u{2717}".red()
    };
    println!("    {} {}", indicator, label);
}

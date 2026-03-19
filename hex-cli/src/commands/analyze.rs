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

pub async fn run(path: &str) -> anyhow::Result<()> {
    let root = Path::new(path)
        .canonicalize()
        .unwrap_or_else(|_| Path::new(path).to_path_buf());

    println!(
        "{} Architecture analysis: {}",
        "\u{2b21}".cyan(),
        root.display()
    );
    println!();

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
                            "  {} Project not registered in nexus",
                            "\u{25cb}".dimmed()
                        );
                        println!(
                            "  {} Use the MCP tool 'hex analyze' for full analysis",
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
            let imports = extract_import_paths(&contents);
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

/// Extract import/use paths from source text (heuristic, not a full parser).
fn extract_import_paths(source: &str) -> Vec<String> {
    let mut paths = Vec::new();
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
                        // Normalize relative path for layer detection
                        let normalized = import_path
                            .replace("../", "")
                            .replace("./", "");
                        paths.push(format!("src/{}", normalized));
                    }
                }
            }
        }
    }
    paths
}

fn print_check(label: &str, present: bool) {
    let indicator = if present {
        "\u{2713}".green()
    } else {
        "\u{2717}".red()
    };
    println!("    {} {}", indicator, label);
}

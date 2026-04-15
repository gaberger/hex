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

#[allow(clippy::too_many_arguments)]
pub async fn run(
    path: &str,
    strict: bool,
    adr_compliance_only: bool,
    json_output: bool,
    file: Option<&str>,
    quiet: bool,
    violations_only: bool,
    exit_code: bool,
) -> anyhow::Result<()> {
    let root = Path::new(path)
        .canonicalize()
        .unwrap_or_else(|_| Path::new(path).to_path_buf());

    // Single-file mode: analyze just one file
    if let Some(file_path) = file {
        return run_single_file(file_path, &root, quiet, violations_only, exit_code);
    }

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

    // Violations collected during boundary analysis (used by violations_only and exit_code)
    let mut local_violations: Vec<boundary::Violation> = Vec::new();
    let mut rust_violations: Vec<RustViolation> = Vec::new();
    let mut all_violation_count = 0usize;

    // If --adr-compliance flag is set, skip boundary analysis entirely
    if !adr_compliance_only {
        // Check for hex project markers
        let has_src = root.join("src").is_dir();
        let has_package_json = root.join("package.json").is_file();
        let has_cargo_toml = root.join("Cargo.toml").is_file();
        let has_go_mod = root.join("go.mod").is_file();
        let has_hex_config = root.join(".hex").is_dir();
        let has_docs_adrs = root.join("docs").join("adrs").is_dir();

        println!("  {}", "Project structure:".bold());
        print_check("src/ directory", has_src);
        print_check("package.json", has_package_json);
        print_check("Cargo.toml", has_cargo_toml);
        print_check("go.mod", has_go_mod);
        print_check(".hex/ config", has_hex_config);
        print_check("docs/adrs/", has_docs_adrs);

        // Check hex architecture layers using hex_core boundary types
        let mut layer_file_counts: Vec<(&str, usize)> = Vec::new();
        if has_src {
            println!();
            println!("  {}", "Hex layers (TypeScript):".bold());

            for (dir, label, expected_layer) in LAYER_DIRS {
                let layer_path = root.join("src").join(dir);
                let exists = layer_path.is_dir();
                let file_count = if exists {
                    collect_source_files(&layer_path).len()
                } else {
                    0
                };

                // Print with file count
                let indicator = if exists { "\u{2713}".green() } else { "\u{2717}".red() };
                if exists {
                    println!("    {} {} ({} files)", indicator, label, file_count);
                } else {
                    println!("    {} {}", indicator, label);
                }
                layer_file_counts.push((label, file_count));

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

        // Rust workspace layer detection (ADR-2603283000)
        let rust_layers = if has_cargo_toml {
            let layers = scan_rust_workspace_layers(&root);
            if !layers.is_empty() {
                println!();
                println!("  {}", "Rust workspace layers:".bold());
                for (label, count) in &layers {
                    let indicator = if *count > 0 { "\u{2713}".green() } else { "\u{2023}".dimmed() };
                    println!("    {} {} ({} files)", indicator, label, count);
                }
            }
            layers
        } else {
            Vec::new()
        };

        // Offline boundary check: scan for obvious violations without nexus
        local_violations = if has_src {
            scan_local_violations(&root)
        } else {
            Vec::new()
        };

        // Rust boundary violations
        rust_violations = if has_cargo_toml {
            scan_rust_boundary_violations(&root)
        } else {
            Vec::new()
        };

        // Go project layer detection
        let go_files_total = if has_go_mod {
            let go_dirs: &[&str] = &["cmd", "internal", "pkg", "api"];
            let mut total = 0usize;
            let has_go_subdirs = go_dirs.iter().any(|d| root.join(d).is_dir());
            if has_go_subdirs {
                println!();
                println!("  {}", "Go project layers:".bold());
                for dir in go_dirs {
                    let layer_path = root.join(dir);
                    if layer_path.is_dir() {
                        let count = collect_source_files(&layer_path).len();
                        total += count;
                        println!("    {} {} ({} files)", "\u{2713}".green(), dir, count);
                    }
                }
            }
            // Also count root-level .go files (flat layout like fizzbuzz)
            let root_go: Vec<_> = std::fs::read_dir(&root)
                .into_iter()
                .flatten()
                .flatten()
                .filter(|e| {
                    e.path().extension().and_then(|x| x.to_str()) == Some("go")
                        && e.path().is_file()
                })
                .collect();
            if !root_go.is_empty() {
                if !has_go_subdirs {
                    println!();
                    println!("  {}", "Go project (flat layout):".bold());
                }
                println!("    {} root-level .go files ({})", "\u{2023}".dimmed(), root_go.len());
                total += root_go.len();
            }
            total
        } else {
            0
        };

        // Count total source files across the project
        let mut total_files = 0usize;
        if has_src {
            total_files += collect_source_files(&root.join("src")).len();
        }
        // Add Rust workspace file counts
        let rust_total: usize = rust_layers.iter().map(|(_, c)| c).sum();
        total_files += rust_total;
        total_files += go_files_total;

        println!();
        println!("  {}", "Boundary analysis:".bold());
        println!("    {} {} source files scanned", "\u{2023}".dimmed(), total_files);

        all_violation_count = local_violations.len() + rust_violations.len();
        if all_violation_count == 0 {
            println!("    {} 0 boundary violations", "\u{2713}".green());
        } else {
            println!(
                "    {} {} boundary violation(s)",
                "\u{26a0}".yellow(),
                all_violation_count
            );
            for v in &local_violations {
                println!(
                    "      {} {} \u{2192} {} ({})",
                    "\u{2717}".red(),
                    v.source_file,
                    v.imported_path,
                    v.rule,
                );
            }
            for v in &rust_violations {
                println!(
                    "      {} {}:{} — {}",
                    "\u{2717}".red(),
                    v.file,
                    v.line,
                    v.message,
                );
            }
        }

        // Try nexus for full boundary analysis + nexus-computed score
        let nexus = NexusClient::from_env();
        let mut nexus_score: Option<u64> = None;
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
                        println!(
                            "    {} Nexus: project registered ({})",
                            "\u{2713}".green(),
                            pid
                        );
                        let health_path = format!("/api/{}/health", pid);
                        if let Ok(health) = nexus.get(&health_path).await {
                            nexus_score = health["score"].as_u64();
                            if let Some(boundary_count) = health["violations"].as_u64() {
                                if boundary_count > 0 {
                                    println!(
                                        "    {} Nexus boundary violations: {}",
                                        "\u{26a0}".yellow(),
                                        boundary_count.to_string().red()
                                    );
                                }
                            }
                        }
                    }
                }
            }
        } else {
            println!(
                "    {} Nexus offline — run {} for deep analysis",
                "\u{25cb}".dimmed(),
                "hex nexus start".dimmed()
            );
        }

        // Compute final score and grade (nexus score takes precedence if available)
        let score = nexus_score.unwrap_or_else(|| {
            let v = all_violation_count as u64;
            if v == 0 { 100 } else { 100u64.saturating_sub(v * 10) }
        });
        let (letter, score_colored) = match score {
            95..=100 => ("A+", format!("{}", score).bright_green().to_string()),
            90..=94  => ("A",  format!("{}", score).green().to_string()),
            80..=89  => ("B",  format!("{}", score).yellow().to_string()),
            70..=79  => ("C",  format!("{}", score).yellow().to_string()),
            60..=69  => ("D",  format!("{}", score).red().to_string()),
            _        => ("F",  format!("{}", score).bright_red().to_string()),
        };

        println!();
        println!(
            "  {} Architecture grade: {} — score {}/100",
            "\u{2b21}".cyan(),
            letter.bold(),
            score_colored,
        );
    }

    // ADR compliance check (ADR-045) — runs locally, no nexus needed
    if !violations_only {
        println!();
        println!("  {}", "ADR compliance:".bold());
    }
    let adr_violations = check_adr_compliance(&root);
    let error_count = adr_violations.iter().filter(|v| v.severity == "error").count();
    let warning_count = adr_violations.iter().filter(|v| v.severity == "warning").count();

    if violations_only {
        // Print only violation lines, no summary
        for v in &local_violations {
            println!(
                "VIOLATION {} \u{2192} {} ({})",
                v.source_file, v.imported_path, v.rule,
            );
        }
        for v in &rust_violations {
            println!(
                "VIOLATION {}:{} — {}",
                v.file, v.line, v.message,
            );
        }
        for v in &adr_violations {
            println!(
                "VIOLATION [{}] {}:{} — {}",
                v.adr, v.file, v.line, v.message,
            );
        }
    } else if adr_violations.is_empty() {
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

    let total_violations = all_violation_count + adr_violations.len();

    // --exit-code: exit 1 if any violations found
    if exit_code && total_violations > 0 {
        std::process::exit(1);
    }

    // --strict: exit with code 1 if any violations exist (warnings promoted to errors)
    if strict && !adr_violations.is_empty() {
        if !violations_only {
            println!();
            println!(
                "  {} --strict mode: {} violation(s) found — exiting with code 1",
                "\u{2717}".red(),
                adr_violations.len(),
            );
        }
        std::process::exit(1);
    }

    Ok(())
}

// ── Single-file analysis (--file flag) ─────────────────────────────────

/// Analyze a single file for hex boundary violations.
/// Used by PostToolUse hooks to check one file at a time.
fn run_single_file(
    file_path: &str,
    root: &Path,
    quiet: bool,
    violations_only: bool,
    exit_code: bool,
) -> anyhow::Result<()> {
    let path = Path::new(file_path);
    let abs_path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir().unwrap_or_else(|_| root.to_path_buf()).join(path)
    };

    if !abs_path.exists() {
        eprintln!("hex analyze --file: file not found: {}", file_path);
        std::process::exit(2);
    }

    // Determine relative path from root for layer detection
    let rel = abs_path
        .strip_prefix(root)
        .unwrap_or(&abs_path)
        .to_string_lossy()
        .to_string();

    let ext = abs_path.extension().and_then(|e| e.to_str()).unwrap_or("");

    let mut violations: Vec<String> = Vec::new();

    match ext {
        "rs" => {
            // Rust boundary check on this single file
            let _src_dir = abs_path.parent().unwrap_or(root);
            // Walk up to find the crate's src/ dir
            let crate_src = find_crate_src_for_file(&abs_path);
            let rel_to_src = if let Some(ref cs) = crate_src {
                abs_path.strip_prefix(cs).unwrap_or(&abs_path).to_string_lossy().to_string()
            } else {
                rel.clone()
            };

            let layer = classify_rust_src_layer(&rel_to_src);
            if let Some(layer_name) = layer {
                let file_rel = abs_path.strip_prefix(root).unwrap_or(&abs_path).to_string_lossy().to_string();
                if let Ok(content) = std::fs::read_to_string(&abs_path) {
                    let mut in_test_section = false;
                    for (idx, line) in content.lines().enumerate() {
                        let trimmed = line.trim();
                        if trimmed == "#[cfg(test)]" { in_test_section = true; }
                        if in_test_section { continue; }
                        if !trimmed.starts_with("use ") { continue; }

                        if matches!(layer_name, "Domain" | "Ports")
                            && (trimmed.contains("::adapters")
                                || trimmed.contains("hex_nexus::")
                                || trimmed.contains("hex_cli::")
                                || trimmed.contains("hex_agent::"))
                            {
                                violations.push(format!(
                                    "{}:{} — {} layer must not import from adapters/downstream: {}",
                                    file_rel, idx + 1, layer_name, trimmed.trim_end_matches(';')
                                ));
                            }
                        if layer_name == "Secondary Adapters" {
                            if let Some(rest) = trimmed.strip_prefix("use crate::adapters::") {
                                let import_mod = rest.split("::").next().unwrap_or("").trim_end_matches(';');
                                let current_mod = rel_to_src
                                    .trim_start_matches("adapters/")
                                    .split('/')
                                    .next()
                                    .unwrap_or("")
                                    .trim_end_matches(".rs");
                                if !import_mod.is_empty()
                                    && import_mod != current_mod
                                    && import_mod != "mod"
                                    && import_mod != "super"
                                {
                                    violations.push(format!(
                                        "{}:{} — Secondary adapter imports sibling '{}': {}",
                                        file_rel, idx + 1, import_mod, trimmed.trim_end_matches(';')
                                    ));
                                }
                            }
                        }
                    }
                }
            }
        }
        "ts" | "js" => {
            // TypeScript/JS boundary check using hex_core
            let source_layer = boundary::detect_layer(&rel);
            if source_layer != Layer::Unknown && source_layer != Layer::CompositionRoot {
                if let Ok(contents) = std::fs::read_to_string(&abs_path) {
                    let imports = extract_import_paths(&contents, &rel);
                    let viols = boundary::validate_imports(&rel, &imports);
                    for v in viols {
                        violations.push(format!(
                            "{} \u{2192} {} ({})",
                            v.source_file, v.imported_path, v.rule
                        ));
                    }
                }
            }
        }
        "go" => {
            // Go boundary check using hex layer conventions
            if let Ok(content) = std::fs::read_to_string(&abs_path) {
                let file_rel = abs_path
                    .strip_prefix(root)
                    .unwrap_or(&abs_path)
                    .to_string_lossy()
                    .to_string();

                // Detect Go module prefix from go.mod for import resolution
                let go_mod_prefix = find_go_module_prefix(root);

                // Classify this file's layer
                let layer_name = classify_go_layer(&file_rel);

                if let Some(layer) = layer_name {
                    for (idx, line) in content.lines().enumerate() {
                        let trimmed = line.trim();
                        // Match Go import lines: "path" or named imports
                        if !trimmed.starts_with('"') && !trimmed.starts_with("//") {
                            continue;
                        }
                        if !trimmed.starts_with('"') {
                            continue;
                        }

                        let import_path = trimmed.trim_matches('"');

                        // Resolve to project-relative path
                        let resolved = if let Some(ref prefix) = go_mod_prefix {
                            if let Some(rest) = import_path.strip_prefix(prefix.as_str()) {
                                rest.strip_prefix('/').unwrap_or(rest).to_string()
                            } else {
                                continue; // stdlib or external — skip
                            }
                        } else {
                            continue; // Can't resolve without go.mod
                        };

                        let target_layer = classify_go_layer(&resolved);

                        // Enforce hex rules
                        if let Some(target) = &target_layer {
                            let violation = match layer.as_str() {
                                "domain" => {
                                    if target != "domain" {
                                        Some(format!("domain must not import {}", target))
                                    } else {
                                        None
                                    }
                                }
                                "ports" => {
                                    if target != "domain" && target != "ports" {
                                        Some(format!("ports must not import {}", target))
                                    } else {
                                        None
                                    }
                                }
                                "adapters" => {
                                    // Check cross-adapter imports
                                    if target == "adapters" && resolved != file_rel {
                                        Some("adapters must not import other adapters".to_string())
                                    } else {
                                        None
                                    }
                                }
                                _ => None,
                            };

                            if let Some(rule) = violation {
                                violations.push(format!(
                                    "{}:{} — {} layer violation: {} (imports {})",
                                    file_rel,
                                    idx + 1,
                                    layer,
                                    rule,
                                    import_path,
                                ));
                            }
                        }
                    }
                }
            }
        }
        _ => {
            // Unsupported extension — nothing to check
        }
    }

    if violations.is_empty() {
        if !quiet && !violations_only {
            println!("\u{2713} {}", file_path);
        }
        return Ok(());
    }

    // Print violations
    for v in &violations {
        println!("VIOLATION {}", v);
    }

    // Single-file mode always exits 1 on violations (designed for hook use)
    let _ = exit_code; // acknowledged — single-file always exits 1 with violations
    std::process::exit(1);
}

/// Walk up from a file to find the enclosing crate's `src/` directory.
fn find_crate_src_for_file(file: &Path) -> Option<PathBuf> {
    let mut dir = file.parent()?;
    loop {
        if dir.join("Cargo.toml").is_file() {
            let src = dir.join("src");
            if src.is_dir() {
                return Some(src);
            }
            return None;
        }
        dir = dir.parent()?;
    }
}

// ── Go Layer Classification ─────────────────────────────────────────────

#[allow(dead_code)]
struct GoLayerRule {
    label: &'static str,
    layer: &'static str,
    signals: &'static [&'static str],
    matches: fn(&str) -> bool,
}

fn match_go_domain(s: &str) -> bool { s.contains("internal/domain") }
fn match_go_ports(s: &str) -> bool { s.contains("internal/ports") }
fn match_go_usecases(s: &str) -> bool { s.contains("internal/usecases") }
fn match_go_adapters(s: &str) -> bool {
    s.contains("internal/adapters") || s.contains("cmd/") || s.contains("pkg/")
}
fn match_go_internal_fallback(s: &str) -> bool { s.contains("internal/") }

static GO_LAYER_RULES: &[GoLayerRule] = &[
    GoLayerRule { label: "domain", layer: "domain", signals: &["internal/domain"], matches: match_go_domain },
    GoLayerRule { label: "ports", layer: "ports", signals: &["internal/ports"], matches: match_go_ports },
    GoLayerRule { label: "usecases", layer: "usecases", signals: &["internal/usecases"], matches: match_go_usecases },
    GoLayerRule { label: "adapters", layer: "adapters", signals: &["internal/adapters", "cmd/", "pkg/"], matches: match_go_adapters },
    GoLayerRule { label: "internal_fallback", layer: "usecases", signals: &["internal/"], matches: match_go_internal_fallback },
];

/// Classify a Go file path into its hexagonal layer.
fn classify_go_layer(path: &str) -> Option<String> {
    GO_LAYER_RULES
        .iter()
        .find(|r| (r.matches)(path))
        .map(|r| r.layer.to_string())
}

/// Read go.mod to extract the module path.
fn find_go_module_prefix(root: &Path) -> Option<String> {
    let go_mod = root.join("go.mod");
    if let Ok(content) = std::fs::read_to_string(go_mod) {
        for line in content.lines() {
            if let Some(rest) = line.strip_prefix("module ") {
                return Some(rest.trim().to_string());
            }
        }
    }
    None
}

// ── Rust Workspace Analysis (ADR-2603283000) ────────────────────────────

/// A boundary violation found in Rust source.
pub struct RustViolation {
    pub file: String,
    pub line: usize,
    pub message: String,
}

/// Find workspace crate directories up to two levels deep.
///
/// Scans direct subdirectories AND their subdirectories for `Cargo.toml` files,
/// so nested workspaces like `spacetime-modules/hexflo-coordination/` are included.
/// Excludes `target/` directories and git worktrees (`hex-worktrees*/`).
fn find_workspace_crate_dirs(root: &Path) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    let Ok(depth1) = std::fs::read_dir(root) else { return dirs; };
    for entry in depth1.flatten() {
        let path = entry.path();
        if !path.is_dir() { continue; }
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if name == "target" || name.starts_with("hex-worktrees") || name.starts_with('.') {
            continue;
        }
        if path.join("Cargo.toml").is_file() {
            dirs.push(path.clone());
        }
        // Also scan one level deeper (e.g. spacetime-modules/hexflo-coordination/)
        if let Ok(depth2) = std::fs::read_dir(&path) {
            for sub in depth2.flatten() {
                let sub_path = sub.path();
                if sub_path.is_dir() && sub_path.join("Cargo.toml").is_file() {
                    let sub_name = sub_path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                    if sub_name != "target" {
                        dirs.push(sub_path);
                    }
                }
            }
        }
    }
    dirs
}

#[allow(dead_code)]
struct RustLayerRule {
    label: &'static str,
    layer: &'static str,
    signals: &'static [&'static str],
    matches: fn(&str) -> bool,
}

fn match_rust_primary(s: &str) -> bool {
    s.starts_with("adapters/primary/") || s.starts_with("adapters/primary.rs")
        || s.starts_with("commands/") || s.starts_with("routes/")
}
fn match_rust_secondary(s: &str) -> bool {
    s.starts_with("adapters/secondary/") || s.starts_with("adapters/secondary.rs")
        || s.starts_with("adapters/")
}
fn match_rust_domain(s: &str) -> bool {
    s.starts_with("domain/") || s.starts_with("domain.rs")
}
fn match_rust_ports(s: &str) -> bool {
    s.starts_with("ports/") || s.starts_with("ports.rs")
}
fn match_rust_usecases(s: &str) -> bool {
    s.starts_with("orchestration/") || s.starts_with("usecases/")
}

static RUST_LAYER_RULES: &[RustLayerRule] = &[
    RustLayerRule { label: "primary_adapters", layer: "Primary Adapters", signals: &["adapters/primary/", "commands/", "routes/"], matches: match_rust_primary },
    RustLayerRule { label: "secondary_adapters", layer: "Secondary Adapters", signals: &["adapters/secondary/", "adapters/"], matches: match_rust_secondary },
    RustLayerRule { label: "domain", layer: "Domain", signals: &["domain/", "domain.rs"], matches: match_rust_domain },
    RustLayerRule { label: "ports", layer: "Ports", signals: &["ports/", "ports.rs"], matches: match_rust_ports },
    RustLayerRule { label: "usecases", layer: "Use Cases", signals: &["orchestration/", "usecases/"], matches: match_rust_usecases },
];

/// Classify a path relative to a crate's `src/` directory into a hex layer label.
/// Returns `None` for infrastructure (unclassified) files.
fn classify_rust_src_layer(rel_to_src: &str) -> Option<&'static str> {
    let p = rel_to_src.replace('\\', "/");
    RUST_LAYER_RULES
        .iter()
        .find(|r| (r.matches)(&p))
        .map(|r| r.layer)
}

/// Scan Rust workspace crates and return layer label → file count aggregated across all crates.
fn scan_rust_workspace_layers(root: &Path) -> Vec<(String, usize)> {
    let crate_dirs = find_workspace_crate_dirs(root);
    let mut counts: std::collections::HashMap<&'static str, usize> = std::collections::HashMap::new();
    let mut infra_count = 0usize;

    for crate_dir in &crate_dirs {
        let src_dir = crate_dir.join("src");
        if !src_dir.is_dir() {
            continue;
        }
        let files = collect_rust_files(&src_dir);
        for file in &files {
            let rel = file
                .strip_prefix(&src_dir)
                .unwrap_or(file)
                .to_string_lossy()
                .to_string();
            match classify_rust_src_layer(&rel) {
                Some(layer) => *counts.entry(layer).or_insert(0) += 1,
                None => infra_count += 1,
            }
        }
    }

    let order = ["Domain", "Ports", "Use Cases", "Primary Adapters", "Secondary Adapters"];
    let mut result: Vec<(String, usize)> = order
        .iter()
        .filter(|&&l| counts.get(l).copied().unwrap_or(0) > 0)
        .map(|&l| (l.to_string(), counts[l]))
        .collect();
    if infra_count > 0 {
        result.push(("Infrastructure".to_string(), infra_count));
    }
    result
}

/// Returns true if the file path is inside a test directory or is a test file.
fn is_test_path(path: &Path) -> bool {
    path.components()
        .any(|c| c.as_os_str() == "tests" || c.as_os_str() == "test")
        || path
            .file_name()
            .and_then(|n| n.to_str())
            .map(|n| n.ends_with("_test.rs") || n.ends_with("_tests.rs"))
            .unwrap_or(false)
}

/// Scan Rust workspace files for hex boundary violations via `use` statement analysis.
fn scan_rust_boundary_violations(root: &Path) -> Vec<RustViolation> {
    let crate_dirs = find_workspace_crate_dirs(root);
    let mut violations = Vec::new();

    for crate_dir in &crate_dirs {
        let src_dir = crate_dir.join("src");
        if !src_dir.is_dir() {
            continue;
        }
        let files = collect_rust_files(&src_dir);
        for file_path in &files {
            if is_test_path(file_path) {
                continue;
            }
            let rel_to_src = file_path
                .strip_prefix(&src_dir)
                .unwrap_or(file_path)
                .to_string_lossy()
                .to_string();
            let Some(layer) = classify_rust_src_layer(&rel_to_src) else {
                continue;
            };
            let file_rel = file_path
                .strip_prefix(root)
                .unwrap_or(file_path)
                .to_string_lossy()
                .to_string();

            let Ok(content) = std::fs::read_to_string(file_path) else {
                continue;
            };

            // Once we see #[cfg(test)] we're in the test section at end of file
            let mut in_test_section = false;
            for (idx, line) in content.lines().enumerate() {
                let trimmed = line.trim();
                if trimmed == "#[cfg(test)]" {
                    in_test_section = true;
                }
                if in_test_section {
                    continue;
                }
                if !trimmed.starts_with("use ") {
                    continue;
                }

                // Rule 1: Domain and Ports must not import from adapters or downstream crates
                if matches!(layer, "Domain" | "Ports")
                    && (trimmed.contains("::adapters")
                        || trimmed.contains("hex_nexus::")
                        || trimmed.contains("hex_cli::")
                        || trimmed.contains("hex_agent::"))
                    {
                        violations.push(RustViolation {
                            file: file_rel.clone(),
                            line: idx + 1,
                            message: format!(
                                "{} layer must not import from adapters/downstream crates: {}",
                                layer,
                                trimmed.trim_end_matches(';')
                            ),
                        });
                    }

                // Rule 2: Secondary adapters must not import sibling secondary adapters
                if layer == "Secondary Adapters" {
                    // use crate::adapters::<sibling>::
                    if let Some(rest) = trimmed.strip_prefix("use crate::adapters::") {
                        let import_mod = rest.split("::").next().unwrap_or("").trim_end_matches(';');
                        // Derive the current file's module name (e.g. "adapters/foo.rs" → "foo")
                        let current_mod = rel_to_src
                            .trim_start_matches("adapters/")
                            .split('/')
                            .next()
                            .unwrap_or("")
                            .trim_end_matches(".rs");
                        if !import_mod.is_empty()
                            && import_mod != current_mod
                            && import_mod != "mod"
                            && import_mod != "super"
                        {
                            violations.push(RustViolation {
                                file: file_rel.clone(),
                                line: idx + 1,
                                message: format!(
                                    "Secondary adapter imports sibling adapter '{}': {}",
                                    import_mod,
                                    trimmed.trim_end_matches(';')
                                ),
                            });
                        }
                    }
                }
            }
        }
    }

    violations
}

/// Collect only `.rs` files recursively under a directory.
fn collect_rust_files(dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    collect_rust_files_recursive(dir, &mut files);
    files
}

fn collect_rust_files_recursive(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_rust_files_recursive(&path, out);
        } else if path.extension().and_then(|x| x.to_str()) == Some("rs") {
            out.push(path);
        }
    }
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
            if matches!(ext, "rs" | "ts" | "js" | "go") {
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
        // Only match lines that start with import/export to avoid false positives
        // from string literals containing import-like text (e.g. template content).
        if trimmed.contains("from")
            && (trimmed.starts_with("import ") || trimmed.starts_with("export "))
        {
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
    /// ADR compliance rules — TOML key `[[adr_rules]]`
    /// (distinct from `[rules]` which is the enforce.rs forbidden-paths table)
    #[serde(default)]
    adr_rules: Vec<AdrRuleConfig>,
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
                    eprintln!(
                        "    {} Loaded {} rule(s) from {}",
                        "\u{2713}".green(),
                        parsed.adr_rules.len(),
                        rules_path.strip_prefix(root).unwrap_or(&rules_path).display(),
                    );
                    parsed.adr_rules
                }
                Err(e) => {
                    eprintln!(
                        "    {} Failed to parse adr-rules.toml: {}",
                        "\u{2717}".red(), e
                    );
                    return Vec::new();
                }
            },
            Err(_) => return Vec::new(),
        }
    } else {
        eprintln!(
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
        // Local boundary violations (TypeScript)
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

        // Rust workspace layers and violations (ADR-2603283000)
        let has_cargo_toml = root.join("Cargo.toml").is_file();
        let rust_layers_data: Vec<serde_json::Value> = if has_cargo_toml {
            scan_rust_workspace_layers(root)
                .iter()
                .map(|(label, count)| serde_json::json!({"layer": label, "file_count": count}))
                .collect()
        } else {
            Vec::new()
        };
        let rust_violations_data: Vec<serde_json::Value> = if has_cargo_toml {
            scan_rust_boundary_violations(root)
                .iter()
                .map(|v| serde_json::json!({"file": v.file, "line": v.line, "message": v.message}))
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

        // Compute local score if nexus didn't provide one
        let total_violations = violations.len() + rust_violations_data.len();
        let final_score = score.unwrap_or_else(|| {
            let v = total_violations as u64;
            if v == 0 { 100 } else { 100u64.saturating_sub(v * 10) }
        });
        result["score"] = serde_json::json!(final_score);
        result["violations"] = serde_json::Value::Array(violations);
        result["boundary_errors"] = serde_json::Value::Array(boundary_errors);
        result["rust_layers"] = serde_json::Value::Array(rust_layers_data);
        result["rust_violations"] = serde_json::Value::Array(rust_violations_data);
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

// ── Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn write_file(base: &std::path::Path, rel: &str, content: &str) {
        let p = base.join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(&p, content).unwrap();
    }

    // ── P5.1: scan_rust_workspace_layers ────────────────────────────────

    #[test]
    fn rust_workspace_detects_domain_and_secondary_adapter() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();

        // Simulate hex-core with domain + ports
        write_file(root, "hex-core/Cargo.toml", "[package]\nname=\"hex-core\"");
        write_file(root, "hex-core/src/domain/mod.rs", "// domain");
        write_file(root, "hex-core/src/domain/tokens.rs", "// tokens");
        write_file(root, "hex-core/src/ports/mod.rs", "// ports");

        // Simulate hex-nexus with adapters
        write_file(root, "hex-nexus/Cargo.toml", "[package]\nname=\"hex-nexus\"");
        write_file(root, "hex-nexus/src/adapters/spacetime.rs", "// adapter");
        write_file(root, "hex-nexus/src/adapters/mod.rs", "// mod");

        let layers = scan_rust_workspace_layers(root);
        let map: std::collections::HashMap<&str, usize> =
            layers.iter().map(|(l, c)| (l.as_str(), *c)).collect();

        assert_eq!(*map.get("Domain").unwrap_or(&0), 2, "expected 2 domain files");
        assert_eq!(*map.get("Ports").unwrap_or(&0), 1, "expected 1 ports file");
        assert_eq!(*map.get("Secondary Adapters").unwrap_or(&0), 2, "expected 2 adapter files");
    }

    #[test]
    fn rust_workspace_infra_crate_no_recognized_dirs() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();

        write_file(root, "hex-parser/Cargo.toml", "[package]\nname=\"hex-parser\"");
        write_file(root, "hex-parser/src/lib.rs", "// parser");
        write_file(root, "hex-parser/src/utils.rs", "// utils");

        let layers = scan_rust_workspace_layers(root);
        let map: std::collections::HashMap<&str, usize> =
            layers.iter().map(|(l, c)| (l.as_str(), *c)).collect();

        assert_eq!(*map.get("Domain").unwrap_or(&0), 0);
        assert_eq!(*map.get("Infrastructure").unwrap_or(&0), 2, "parser files should be infrastructure");
    }

    #[test]
    fn rust_workspace_empty_root_returns_empty() {
        let tmp = tempfile::tempdir().unwrap();
        let layers = scan_rust_workspace_layers(tmp.path());
        assert!(layers.is_empty());
    }

    #[test]
    fn rust_workspace_primary_adapter_commands_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();

        write_file(root, "hex-cli/Cargo.toml", "[package]\nname=\"hex-cli\"");
        write_file(root, "hex-cli/src/commands/analyze.rs", "// analyze");
        write_file(root, "hex-cli/src/commands/plan.rs", "// plan");

        let layers = scan_rust_workspace_layers(root);
        let map: std::collections::HashMap<&str, usize> =
            layers.iter().map(|(l, c)| (l.as_str(), *c)).collect();

        assert_eq!(*map.get("Primary Adapters").unwrap_or(&0), 2);
    }

    // ── P5.2: scan_rust_boundary_violations ─────────────────────────────

    #[test]
    fn rust_boundary_domain_importing_adapters_is_violation() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();

        write_file(root, "my-crate/Cargo.toml", "[package]\nname=\"my-crate\"");
        write_file(
            root,
            "my-crate/src/domain/bad.rs",
            "use hex_nexus::adapters::spacetime;\npub fn foo() {}",
        );

        let violations = scan_rust_boundary_violations(root);
        assert_eq!(violations.len(), 1);
        assert!(violations[0].message.contains("Domain layer must not import"));
    }

    #[test]
    fn rust_boundary_secondary_adapter_importing_sibling_is_violation() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();

        write_file(root, "my-crate/Cargo.toml", "[package]\nname=\"my-crate\"");
        write_file(
            root,
            "my-crate/src/adapters/foo.rs",
            "use crate::adapters::bar::BarClient;\npub fn run() {}",
        );

        let violations = scan_rust_boundary_violations(root);
        assert_eq!(violations.len(), 1);
        assert!(violations[0].message.contains("sibling adapter"));
    }

    #[test]
    fn rust_boundary_use_in_cfg_test_is_ignored() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();

        write_file(root, "my-crate/Cargo.toml", "[package]\nname=\"my-crate\"");
        write_file(
            root,
            "my-crate/src/domain/clean.rs",
            "pub fn foo() {}\n\n#[cfg(test)]\nmod tests {\n    use hex_nexus::adapters::mock;\n}",
        );

        let violations = scan_rust_boundary_violations(root);
        assert!(violations.is_empty(), "test-section imports must not be flagged");
    }

    #[test]
    fn rust_boundary_clean_file_produces_no_violations() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();

        write_file(root, "my-crate/Cargo.toml", "[package]\nname=\"my-crate\"");
        write_file(
            root,
            "my-crate/src/domain/clean.rs",
            "use std::collections::HashMap;\npub struct Foo { pub x: u32 }",
        );
        write_file(
            root,
            "my-crate/src/adapters/clean.rs",
            "use hex_core::ports::IFooPort;\npub struct FooAdapter;",
        );

        let violations = scan_rust_boundary_violations(root);
        assert!(violations.is_empty());
    }

    // ── P5.3: classify_rust_src_layer ───────────────────────────────────

    #[test]
    fn classify_layer_maps_known_paths() {
        assert_eq!(classify_rust_src_layer("domain/tokens.rs"), Some("Domain"));
        assert_eq!(classify_rust_src_layer("ports/inference.rs"), Some("Ports"));
        assert_eq!(classify_rust_src_layer("adapters/spacetime.rs"), Some("Secondary Adapters"));
        assert_eq!(classify_rust_src_layer("adapters/primary/cli.rs"), Some("Primary Adapters"));
        assert_eq!(classify_rust_src_layer("commands/analyze.rs"), Some("Primary Adapters"));
        assert_eq!(classify_rust_src_layer("routes/chat.rs"), Some("Primary Adapters"));
        assert_eq!(classify_rust_src_layer("orchestration/agent_manager.rs"), Some("Use Cases"));
        assert_eq!(classify_rust_src_layer("lib.rs"), None);
        assert_eq!(classify_rust_src_layer("main.rs"), None);
    }

    // ── P5.3: smoke test — zero violations on clean hex-intf ────────────

    #[test]
    fn rust_boundary_zero_violations_on_hex_intf() {
        // Find the repo root (two levels up from hex-cli/src/commands/)
        let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR")); // hex-cli/
        let root = manifest.parent().unwrap(); // hex-intf/

        let violations = scan_rust_boundary_violations(root);
        if !violations.is_empty() {
            for v in &violations {
                eprintln!("VIOLATION {}:{} — {}", v.file, v.line, v.message);
            }
        }
        assert!(
            violations.is_empty(),
            "{} Rust boundary violation(s) found in hex-intf — these are real bugs",
            violations.len()
        );
    }

    #[test]
    fn go_layer_rule_table_invariants() {
        assert_eq!(GO_LAYER_RULES.len(), 5, "expected 5 Go layer rules");
        for rule in GO_LAYER_RULES {
            assert!(!rule.label.is_empty());
            assert!(!rule.signals.is_empty(), "rule {:?} has no signals", rule.label);
        }
        let domain_idx = GO_LAYER_RULES.iter().position(|r| r.label == "domain").unwrap();
        let fallback_idx = GO_LAYER_RULES.iter().position(|r| r.label == "internal_fallback").unwrap();
        assert!(domain_idx < fallback_idx,
            "specific internal/ rules must precede internal_fallback");
    }

    #[test]
    fn rust_layer_rule_table_invariants() {
        assert_eq!(RUST_LAYER_RULES.len(), 5, "expected 5 Rust layer rules");
        for rule in RUST_LAYER_RULES {
            assert!(!rule.label.is_empty());
            assert!(!rule.signals.is_empty(), "rule {:?} has no signals", rule.label);
        }
        let primary_idx = RUST_LAYER_RULES.iter().position(|r| r.label == "primary_adapters").unwrap();
        let secondary_idx = RUST_LAYER_RULES.iter().position(|r| r.label == "secondary_adapters").unwrap();
        assert!(primary_idx < secondary_idx,
            "primary_adapters must precede secondary_adapters (adapters/ fallback)");
    }
}

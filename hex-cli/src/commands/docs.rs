//! `hex docs` — internal-documentation health (ADR-047 Phase 4).
//!
//! `hex docs check` scans the docs/ tree and `spacetime-modules/*/README.md`
//! for:
//!   - stale terminology (canonical glossary from ADR-047 §"Glossary"),
//!   - missing per-module READMEs in `spacetime-modules/`,
//!   - docs older than `--max-age-days` (default 90) since last git commit.
//!
//! Exit codes (mirrors `hex adr doctor`): 0 = clean, 1 = warnings only,
//! 2 = errors (or any finding under `--strict`).

use std::path::{Path, PathBuf};

use clap::Subcommand;
use colored::Colorize;

#[derive(Subcommand)]
pub enum DocsAction {
    /// Run all freshness + terminology checks across docs/ and module READMEs.
    Check {
        /// Emit findings as JSON (`{findings: [...], summary: {...}}`).
        #[arg(long)]
        json: bool,
        /// Treat warnings as errors (exit 2 on any finding).
        #[arg(long)]
        strict: bool,
        /// Max age in days before a doc is flagged as stale (default 90; 0 = disabled).
        #[arg(long, default_value_t = 90)]
        max_age_days: i64,
        /// Path to scan (default: discovered project root).
        #[arg(long, value_name = "PATH")]
        root: Option<String>,
    },
    /// List the canonical glossary terms enforced by `check`.
    Glossary,
    /// Add YAML frontmatter to one ADR (ADR-047 Phase 5). Idempotent; skips ADRs
    /// that already have a `---` block. Preview by default — pass `--apply` to write.
    MigrateAdr {
        /// ADR ID (e.g. `ADR-047`) or path to an ADR file.
        adr: String,
        /// Write the migration to disk (default: dry-run preview).
        #[arg(long)]
        apply: bool,
    },
}

pub async fn run(action: DocsAction) -> anyhow::Result<()> {
    match action {
        DocsAction::Check {
            json,
            strict,
            max_age_days,
            root,
        } => check(json, strict, max_age_days, root).await,
        DocsAction::Glossary => {
            print_glossary();
            Ok(())
        }
        DocsAction::MigrateAdr { adr, apply } => migrate_adr(&adr, apply).await,
    }
}

// ─── Canonical glossary (ADR-047) ──────────────────────────────────────────

/// (deprecated_term, canonical_term, severity).
///
/// `severity = "error"` flips a warning into an error. Use it for
/// terms that aren't merely sloppy but actively confuse readers
/// (e.g. legacy product names that have been replaced).
const GLOSSARY: &[(&str, &str, &str)] = &[
    // Legacy product names
    ("hex-hub", "hex-nexus", "error"),
    ("ruflo", "HexFlo", "error"),
    // Sloppy abbreviations that lose meaning
    ("orchestration nexus", "hex-nexus", "warning"),
];

fn print_glossary() {
    println!("{} Canonical glossary terms (ADR-047)", "\u{2b21}".cyan());
    println!();
    println!("  {:<25} {:<20} {}", "Deprecated".bold(), "Canonical".bold(), "Severity".bold());
    for (bad, good, sev) in GLOSSARY {
        let sev_str = match *sev {
            "error" => sev.red().to_string(),
            "warning" => sev.yellow().to_string(),
            _ => sev.normal().to_string(),
        };
        println!("  {:<25} {:<20} {}", bad, good, sev_str);
    }
    println!();
    println!(
        "  {} Source: docs/adrs/ADR-047-internal-documentation-system.md",
        "\u{2139}".dimmed()
    );
}

// ─── Findings ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, serde::Serialize)]
struct Finding {
    kind: String,
    severity: String,
    path: String,
    line: Option<usize>,
    detail: String,
}

#[derive(Debug, Clone, serde::Serialize)]
struct Summary {
    total: usize,
    errors: usize,
    warnings: usize,
}

// ─── Project root discovery ────────────────────────────────────────────────

fn discover_root(explicit: Option<String>) -> anyhow::Result<PathBuf> {
    if let Some(p) = explicit {
        return Ok(PathBuf::from(p));
    }
    let mut dir = std::env::current_dir()?;
    loop {
        if dir.join("docs").is_dir() || dir.join(".git").is_dir() {
            return Ok(dir);
        }
        match dir.parent() {
            Some(parent) => dir = parent.to_path_buf(),
            None => anyhow::bail!("could not find project root (no docs/ or .git/ ancestor)"),
        }
    }
}

// ─── Check entrypoint ──────────────────────────────────────────────────────

async fn check(
    json: bool,
    strict: bool,
    max_age_days: i64,
    root: Option<String>,
) -> anyhow::Result<()> {
    let root = discover_root(root)?;
    let mut findings: Vec<Finding> = Vec::new();

    findings.extend(check_module_readmes(&root)?);
    findings.extend(check_terminology(&root)?);
    if max_age_days > 0 {
        findings.extend(check_staleness(&root, max_age_days)?);
    }

    let errors = findings.iter().filter(|f| f.severity == "error").count();
    let warnings = findings.iter().filter(|f| f.severity == "warning").count();
    let summary = Summary {
        total: findings.len(),
        errors,
        warnings,
    };

    if json {
        let payload = serde_json::json!({
            "findings": findings,
            "summary": summary,
        });
        println!("{}", serde_json::to_string_pretty(&payload)?);
    } else {
        print_human(&findings, &summary, &root);
    }

    let code = exit_code(errors, warnings, strict);
    if code != 0 {
        std::process::exit(code);
    }
    Ok(())
}

fn exit_code(errors: usize, warnings: usize, strict: bool) -> i32 {
    if errors > 0 {
        2
    } else if strict && warnings > 0 {
        2
    } else if warnings > 0 {
        1
    } else {
        0
    }
}

fn print_human(findings: &[Finding], summary: &Summary, root: &Path) {
    println!("{} Documentation health (ADR-047)", "\u{2b21}".cyan());
    println!("  root: {}", root.display());
    println!();
    if findings.is_empty() {
        println!("  {}", "No findings — docs look healthy.".green());
        return;
    }
    for f in findings {
        let sev = match f.severity.as_str() {
            "error" => "ERROR".red().to_string(),
            "warning" => "WARN".yellow().to_string(),
            other => other.normal().to_string(),
        };
        let loc = match f.line {
            Some(n) => format!("{}:{}", f.path, n),
            None => f.path.clone(),
        };
        println!("  [{}] {} ({}) — {}", sev, loc.bold(), f.kind, f.detail);
    }
    println!();
    println!(
        "  {} finding(s) — {} error(s), {} warning(s)",
        summary.total, summary.errors, summary.warnings
    );
}

// ─── Module README check ───────────────────────────────────────────────────

fn check_module_readmes(root: &Path) -> anyhow::Result<Vec<Finding>> {
    let mods_dir = root.join("spacetime-modules");
    if !mods_dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for entry in std::fs::read_dir(&mods_dir)? {
        let entry = entry?;
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        // Only count directories that look like a Rust crate.
        if !path.join("Cargo.toml").exists() {
            continue;
        }
        if !path.join("README.md").exists() {
            let rel = relative_to(&path, root);
            out.push(Finding {
                kind: "missing_module_readme".into(),
                severity: "error".into(),
                path: rel,
                line: None,
                detail: "WASM module has no README.md (ADR-047 Phase 3)".into(),
            });
        }
    }
    Ok(out)
}

// ─── Terminology check ─────────────────────────────────────────────────────

fn check_terminology(root: &Path) -> anyhow::Result<Vec<Finding>> {
    let mut out = Vec::new();
    for path in collect_md_files(root)? {
        if is_terminology_authority(&path) {
            continue;
        }
        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => continue,
        };
        for (line_no, line) in content.lines().enumerate() {
            let lower = line.to_lowercase();
            for (bad, good, sev) in GLOSSARY {
                if lower.contains(bad) {
                    out.push(Finding {
                        kind: "stale_terminology".into(),
                        severity: (*sev).into(),
                        path: relative_to(&path, root),
                        line: Some(line_no + 1),
                        detail: format!("'{}' is deprecated; use '{}'", bad, good),
                    });
                }
            }
        }
    }
    Ok(out)
}

// ─── Staleness check (git-based) ───────────────────────────────────────────

fn check_staleness(root: &Path, max_age_days: i64) -> anyhow::Result<Vec<Finding>> {
    let mut out = Vec::new();
    for path in collect_md_files(root)? {
        // Module READMEs we just authored aren't tracked yet — skip files that
        // aren't in git, since git-log returns nothing and we'd false-positive.
        let age = match git_last_commit_age_days(&path) {
            Some(age) => age,
            None => continue,
        };
        if age > max_age_days {
            out.push(Finding {
                kind: "stale_doc".into(),
                severity: "warning".into(),
                path: relative_to(&path, root),
                line: None,
                detail: format!(
                    "no git commit in {} days (threshold {})",
                    age, max_age_days
                ),
            });
        }
    }
    Ok(out)
}

fn git_last_commit_age_days(path: &Path) -> Option<i64> {
    let out = std::process::Command::new("git")
        .args(["log", "-1", "--format=%ct", "--", &path.to_string_lossy()])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    let ts: i64 = stdout.trim().parse().ok()?;
    let now = chrono::Utc::now().timestamp();
    Some(((now - ts).max(0)) / 86400)
}

// ─── Helpers ───────────────────────────────────────────────────────────────

fn collect_md_files(root: &Path) -> anyhow::Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    // Roots we care about: docs/, spacetime-modules/, top-level README/CLAUDE.
    let candidates = [
        root.join("docs"),
        root.join("spacetime-modules"),
    ];
    for base in &candidates {
        if base.is_dir() {
            walk_md(base, &mut out);
        }
    }
    for fname in ["README.md", "CLAUDE.md"] {
        let p = root.join(fname);
        if p.is_file() {
            out.push(p);
        }
    }
    Ok(out)
}

fn walk_md(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        // Skip target/, node_modules/, .git/
        if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
            if matches!(name, "target" | "node_modules" | ".git" | "dist" | "drafts") {
                continue;
            }
        }
        if path.is_dir() {
            walk_md(&path, out);
        } else if path.extension().and_then(|e| e.to_str()) == Some("md") {
            out.push(path);
        }
    }
}

/// Files whose role IS to enumerate banned terms — checking them would
/// always false-positive. ADRs are excluded wholesale because they cite
/// historical names when describing the migration.
fn is_terminology_authority(path: &Path) -> bool {
    if path
        .components()
        .any(|c| c.as_os_str() == "adrs")
    {
        return true;
    }
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_lowercase();
    matches!(name.as_str(), "glossary.md" | "banned-terms.md")
}

fn relative_to(path: &Path, root: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exit_code_clean_is_zero() {
        assert_eq!(exit_code(0, 0, false), 0);
    }

    #[test]
    fn exit_code_warning_is_one() {
        assert_eq!(exit_code(0, 3, false), 1);
    }

    #[test]
    fn exit_code_error_is_two() {
        assert_eq!(exit_code(1, 0, false), 2);
    }

    #[test]
    fn exit_code_strict_promotes_warning_to_two() {
        assert_eq!(exit_code(0, 1, true), 2);
    }

    #[test]
    fn glossary_has_known_legacy_terms() {
        let bad_terms: Vec<&str> = GLOSSARY.iter().map(|(b, _, _)| *b).collect();
        assert!(bad_terms.contains(&"hex-hub"));
        assert!(bad_terms.contains(&"ruflo"));
    }
}

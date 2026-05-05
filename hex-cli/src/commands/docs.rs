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
    findings.extend(check_adr_frontmatter(&root)?);
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

// ─── ADR frontmatter audit (Phase 5) ──────────────────────────────────────

fn check_adr_frontmatter(root: &Path) -> anyhow::Result<Vec<Finding>> {
    let adrs_dir = root.join("docs").join("adrs");
    if !adrs_dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for entry in std::fs::read_dir(&adrs_dir)? {
        let entry = entry?;
        let path = entry.path();
        let name = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n,
            None => continue,
        };
        if !name.starts_with("ADR-") || !name.ends_with(".md") {
            continue;
        }
        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => continue,
        };
        if !has_yaml_frontmatter(&content) {
            out.push(Finding {
                kind: "missing_adr_frontmatter".into(),
                severity: "warning".into(),
                path: relative_to(&path, root),
                line: Some(1),
                detail:
                    "ADR has no YAML frontmatter (ADR-047 Phase 5). Run `hex docs migrate-adr <id>`."
                        .into(),
            });
        }
    }
    Ok(out)
}

fn has_yaml_frontmatter(content: &str) -> bool {
    let mut lines = content.lines();
    if lines.next().map(|l| l.trim()) != Some("---") {
        return false;
    }
    // A frontmatter block must close with another `---` on its own line.
    lines.any(|l| l.trim() == "---")
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

// ─── ADR frontmatter migration (Phase 5) ──────────────────────────────────

async fn migrate_adr(adr: &str, apply: bool) -> anyhow::Result<()> {
    let path = resolve_adr_path(adr)?;
    let content = std::fs::read_to_string(&path)?;

    if has_yaml_frontmatter(&content) {
        println!(
            "{} {} already has YAML frontmatter — nothing to migrate.",
            "\u{2713}".green(),
            path.display()
        );
        return Ok(());
    }

    let frontmatter = synthesize_frontmatter(&path, &content);
    let new_content = format!("{}\n{}", frontmatter, content);

    if apply {
        std::fs::write(&path, &new_content)?;
        println!(
            "{} Wrote frontmatter to {}",
            "\u{2713}".green(),
            path.display()
        );
    } else {
        println!(
            "{} Preview for {} (run with --apply to write):",
            "\u{2b21}".cyan(),
            path.display()
        );
        println!();
        for line in frontmatter.lines() {
            println!("{} {}", "+".green(), line);
        }
        println!();
        println!(
            "{} {} line(s) will be prepended.",
            "\u{2139}".dimmed(),
            frontmatter.lines().count()
        );
    }
    Ok(())
}

fn resolve_adr_path(adr: &str) -> anyhow::Result<PathBuf> {
    // If it's a path that exists, use it directly.
    let direct = PathBuf::from(adr);
    if direct.is_file() {
        return Ok(direct);
    }
    let root = discover_root(None)?;
    let adrs = root.join("docs").join("adrs");
    if !adrs.is_dir() {
        anyhow::bail!("docs/adrs/ not found under {}", root.display());
    }
    let needle = adr.to_uppercase();
    let needle = if needle.starts_with("ADR-") {
        needle
    } else {
        format!("ADR-{}", needle)
    };
    for entry in std::fs::read_dir(&adrs)? {
        let entry = entry?;
        let p = entry.path();
        let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if name.to_uppercase().starts_with(&format!("{}-", needle))
            || name.to_uppercase() == format!("{}.MD", needle)
        {
            return Ok(p);
        }
    }
    anyhow::bail!("no ADR matched '{}' under {}", adr, adrs.display())
}

/// Synthesize a YAML frontmatter block from the markdown-style fields the ADRs
/// already use. Idempotent on output (sorted, normalized).
fn synthesize_frontmatter(path: &Path, content: &str) -> String {
    let id = adr_id_from_filename(path);
    let status = parse_field(content, "status").unwrap_or_else(|| "Proposed".into());
    let date = parse_field(content, "date")
        .or_else(|| parse_field(content, "accepted date"))
        .unwrap_or_else(|| chrono::Utc::now().format("%Y-%m-%d").to_string());
    let supersedes = parse_csv_field(content, "supersedes");
    let superseded_by = parse_field(content, "superseded by")
        .or_else(|| parse_field(content, "superseded-by"));
    let depends_on = parse_csv_field(content, "depends on")
        .into_iter()
        .chain(parse_csv_field(content, "affects"))
        .collect::<Vec<_>>();

    let mut buf = String::new();
    buf.push_str("---\n");
    buf.push_str(&format!("id: {}\n", id));
    buf.push_str(&format!("status: {}\n", status.to_lowercase()));
    buf.push_str(&format!("date: {}\n", date));
    buf.push_str(&format!("supersedes: {}\n", yaml_list(&supersedes)));
    buf.push_str(&format!(
        "superseded_by: {}\n",
        match superseded_by.as_deref() {
            Some(s) if !s.is_empty() => s,
            _ => "null",
        }
    ));
    buf.push_str(&format!("depends_on: {}\n", yaml_list(&depends_on)));
    buf.push_str("components: []\n");
    buf.push_str("modules: []\n");
    buf.push_str("---\n");
    buf
}

fn yaml_list(items: &[String]) -> String {
    if items.is_empty() {
        return "[]".into();
    }
    let parts: Vec<String> = items
        .iter()
        .filter(|s| !s.is_empty())
        .map(|s| s.clone())
        .collect();
    format!("[{}]", parts.join(", "))
}

fn adr_id_from_filename(path: &Path) -> String {
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("");
    let rest = stem.strip_prefix("ADR-").unwrap_or(stem);
    let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() {
        stem.to_string()
    } else {
        format!("ADR-{}", digits)
    }
}

/// Parse a single-value field from common ADR markdown styles.
/// Matches `**<name>:** value`, `## <name>: value`, or `<name>: value`
/// (case-insensitive). Lines inside fenced code blocks (```...```) are
/// skipped so embedded YAML examples don't pollute the synthesized output.
fn parse_field(content: &str, name: &str) -> Option<String> {
    let lname = name.to_lowercase();
    let bold_prefix = format!("**{}:**", lname);
    let heading_prefix = format!("## {}:", lname);
    let plain_prefix = format!("{}:", lname);
    let mut in_fence = false;
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("```") {
            in_fence = !in_fence;
            continue;
        }
        if in_fence {
            continue;
        }
        let lower = trimmed.to_lowercase();
        if lower.starts_with(&bold_prefix) {
            return Some(trimmed[bold_prefix.len()..].trim().to_string());
        }
        if lower.starts_with(&heading_prefix) {
            return Some(trimmed[heading_prefix.len()..].trim().to_string());
        }
        if lower.starts_with(&plain_prefix) && !lower.starts_with("**") && !lower.starts_with("##") {
            return Some(trimmed[plain_prefix.len()..].trim().to_string());
        }
    }
    None
}

/// Parse a comma-separated field into a list of trimmed values.
fn parse_csv_field(content: &str, name: &str) -> Vec<String> {
    parse_field(content, name)
        .map(|raw| {
            raw.split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_default()
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

    #[test]
    fn detects_yaml_frontmatter() {
        assert!(has_yaml_frontmatter("---\nid: ADR-001\n---\n# title"));
        assert!(!has_yaml_frontmatter("# title\n\n**Status:** Accepted\n"));
        // Open-ended `---` that never closes is not frontmatter.
        assert!(!has_yaml_frontmatter("---\nid: ADR-001\n"));
    }

    #[test]
    fn parse_bold_field() {
        let content = "# ADR-001\n**Status:** Accepted\n**Date:** 2026-03-15\n";
        assert_eq!(parse_field(content, "status").as_deref(), Some("Accepted"));
        assert_eq!(parse_field(content, "date").as_deref(), Some("2026-03-15"));
    }

    #[test]
    fn parse_csv_supersedes() {
        let content = "**Supersedes:** ADR-016, ADR-017\n";
        assert_eq!(
            parse_csv_field(content, "supersedes"),
            vec!["ADR-016".to_string(), "ADR-017".to_string()]
        );
    }

    #[test]
    fn synthesize_frontmatter_basic() {
        let path = std::path::Path::new("ADR-047-foo.md");
        let content = "# ADR-047: Foo\n\n**Status:** Accepted\n**Date:** 2026-03-22\n";
        let fm = synthesize_frontmatter(path, content);
        assert!(fm.starts_with("---\n"));
        assert!(fm.ends_with("---\n"));
        assert!(fm.contains("id: ADR-047"));
        assert!(fm.contains("status: accepted"));
        assert!(fm.contains("date: 2026-03-22"));
        assert!(fm.contains("components: []"));
        assert!(fm.contains("modules: []"));
    }

    #[test]
    fn yaml_list_empty_and_full() {
        assert_eq!(yaml_list(&[]), "[]");
        assert_eq!(
            yaml_list(&["ADR-016".into(), "ADR-017".into()]),
            "[ADR-016, ADR-017]"
        );
    }

    #[test]
    fn adr_id_extraction_handles_legacy_and_timestamp() {
        let p = std::path::Path::new("docs/adrs/ADR-047-internal-docs.md");
        assert_eq!(adr_id_from_filename(p), "ADR-047");
        let p = std::path::Path::new("docs/adrs/ADR-2603221500-foo.md");
        assert_eq!(adr_id_from_filename(p), "ADR-2603221500");
    }
}

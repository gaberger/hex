//! `hex plan lint` — authoring-time workplan validation (wp-enforce-workplan-evidence E3.1).
//!
//! Front-end for `schema_validate::validate_workplan_evidence`. Designed to be
//! called from three places: (a) CLI `hex plan lint <file>` for author feedback,
//! (b) CLI `hex plan lint --all` for bulk audit, (c) pre-commit hook (E3.2) that
//! blocks commits touching `docs/workplans/wp-*.json` when violations exist.
//!
//! Exit code discipline:
//! - 0 = all checked workplans pass
//! - 1 = at least one violation found
//! - 2 = I/O or parse error (distinct from "validated but failed")
//!
//! Output is a single table per file with violating-task rows only. Clean
//! files print a one-line confirmation. This keeps the hook output quiet on
//! the happy path.

use colored::*;
use std::path::{Path, PathBuf};

use super::{schema_validate, Workplan};

/// Entry point for `PlanAction::Lint`.
pub(super) async fn run(file: Option<&str>, all: bool) -> anyhow::Result<()> {
    let targets = resolve_targets(file, all)?;
    if targets.is_empty() {
        println!("{} no workplans found to lint", "⬡".cyan());
        return Ok(());
    }

    let mut total_violations = 0usize;
    let mut files_with_violations = 0usize;

    for path in &targets {
        match lint_one(path) {
            Ok(0) => {
                if targets.len() == 1 {
                    println!("{} {}: clean", "✓".green(), path.display());
                }
            }
            Ok(n) => {
                total_violations += n;
                files_with_violations += 1;
            }
            Err(e) => {
                eprintln!("{} {}: {}", "✗".red(), path.display(), e);
                std::process::exit(2);
            }
        }
    }

    if total_violations == 0 {
        if targets.len() > 1 {
            println!(
                "{} all {} workplans clean",
                "✓".green(),
                targets.len()
            );
        }
        Ok(())
    } else {
        eprintln!(
            "\n{} {} violation(s) across {} file(s)",
            "✗".red(),
            total_violations,
            files_with_violations
        );
        std::process::exit(1);
    }
}

/// Decide which workplan paths to lint. --all overrides any file argument.
fn resolve_targets(file: Option<&str>, all: bool) -> anyhow::Result<Vec<PathBuf>> {
    if all {
        let dir = PathBuf::from("docs/workplans");
        if !dir.is_dir() {
            anyhow::bail!(
                "docs/workplans/ not found in current directory (run from repo root)"
            );
        }
        let mut out = Vec::new();
        for entry in std::fs::read_dir(&dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) == Some("json") {
                let name = path
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or_default();
                if name.starts_with("wp-") {
                    out.push(path);
                }
            }
        }
        out.sort();
        Ok(out)
    } else if let Some(f) = file {
        let path = PathBuf::from(f);
        if !path.is_file() {
            anyhow::bail!("workplan not found: {}", path.display());
        }
        Ok(vec![path])
    } else {
        anyhow::bail!("pass a workplan path or --all");
    }
}

/// Lint a single workplan. Returns number of violations found.
fn lint_one(path: &Path) -> anyhow::Result<usize> {
    let content = std::fs::read_to_string(path)?;
    let wp: Workplan = serde_json::from_str(&content)?;
    match schema_validate::validate_workplan_evidence(&wp) {
        Ok(()) => Ok(0),
        Err(violations) => {
            let n = violations.len();
            eprintln!(
                "\n{} {} — {} violation(s):",
                "✗".red(),
                path.display(),
                n
            );
            for v in &violations {
                eprintln!("  - {}", v);
            }
            Ok(n)
        }
    }
}

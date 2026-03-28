use std::path::{Path, PathBuf};

use clap::Subcommand;
use colored::Colorize;
use serde::Deserialize;
use tabled::Tabled;

use crate::fmt::{status_badge, truncate, HexTable};

#[derive(Subcommand)]
pub enum SpecAction {
    /// List all behavioral specs in docs/specs/
    List,
    /// Show all scenarios for a feature spec
    Show {
        /// Feature name (e.g. declarative-swarm-agents) or partial match
        feature: String,
        /// Check whether implementation exists for each spec category and function
        #[arg(long)]
        check: bool,
    },
    /// Show which workplan(s) a spec feeds into
    Workplan {
        /// Feature name (partial match supported)
        feature: String,
    },
}

pub async fn run(action: SpecAction) -> anyhow::Result<()> {
    match action {
        SpecAction::List => list().await,
        SpecAction::Show { feature, check } => show(&feature, check).await,
        SpecAction::Workplan { feature } => workplan(&feature).await,
    }
}

// ── Serde structs ────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct SpecFile {
    feature: String,
    #[serde(default)]
    description: String,
    specs: Vec<SpecScenario>,
}

#[derive(Deserialize)]
struct SpecScenario {
    id: String,
    #[serde(default)]
    category: String,
    description: String,
    #[serde(default)]
    given: String,
    #[serde(default)]
    when: String,
    #[serde(default)]
    then: String,
    #[serde(default)]
    negative_spec: bool,
}

#[derive(Deserialize)]
pub(crate) struct WorkplanFile {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub adr: String,
    // May be a path string or absent
    #[serde(default)]
    pub specs: Option<serde_json::Value>,
    #[serde(default)]
    pub status: String,
}

// ── Path discovery ───────────────────────────────────────────────────────────

pub(crate) fn find_specs_dir() -> Option<PathBuf> {
    let cwd = std::env::current_dir().ok()?;
    let mut dir = cwd.as_path();
    loop {
        let candidate = dir.join("docs").join("specs");
        if candidate.is_dir() {
            return Some(candidate);
        }
        dir = dir.parent()?;
    }
}

pub(crate) fn find_workplans_dir() -> Option<PathBuf> {
    let cwd = std::env::current_dir().ok()?;
    let mut dir = cwd.as_path();
    loop {
        let candidate = dir.join("docs").join("workplans");
        if candidate.is_dir() {
            return Some(candidate);
        }
        dir = dir.parent()?;
    }
}

async fn collect_specs(dir: &Path) -> anyhow::Result<Vec<(PathBuf, SpecFile)>> {
    let mut result = Vec::new();
    let mut entries = tokio::fs::read_dir(dir).await?;
    while let Some(entry) = entries.next_entry().await? {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("json") {
            let raw = tokio::fs::read_to_string(&path).await?;
            if let Ok(spec) = serde_json::from_str::<SpecFile>(&raw) {
                result.push((path, spec));
            }
        }
    }
    result.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(result)
}

pub(crate) async fn collect_workplans(dir: &Path) -> anyhow::Result<Vec<(PathBuf, WorkplanFile)>> {
    let mut result = Vec::new();
    let mut entries = tokio::fs::read_dir(dir).await?;
    while let Some(entry) = entries.next_entry().await? {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("json")
            && path.file_name().and_then(|f| f.to_str()).map(|n| !n.starts_with('_')).unwrap_or(false)
        {
            let raw = tokio::fs::read_to_string(&path).await?;
            if let Ok(wp) = serde_json::from_str::<WorkplanFile>(&raw) {
                result.push((path, wp));
            }
        }
    }
    result.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(result)
}

/// Resolve the specs path from a workplan's "specs" field value.
/// Handles: string path, or absent.
pub(crate) fn workplan_specs_path(val: &serde_json::Value) -> Option<String> {
    match val {
        serde_json::Value::String(s) => Some(s.clone()),
        _ => None,
    }
}

// ── Tabled rows ──────────────────────────────────────────────────────────────

#[derive(Tabled)]
struct SpecListRow {
    #[tabled(rename = "Feature")]
    feature: String,
    #[tabled(rename = "Scenarios")]
    count: usize,
    #[tabled(rename = "Description")]
    description: String,
}

#[derive(Tabled)]
struct WorkplanLinkRow {
    #[tabled(rename = "Workplan")]
    id: String,
    #[tabled(rename = "ADR")]
    adr: String,
    #[tabled(rename = "Status")]
    status: String,
    #[tabled(rename = "Title")]
    title: String,
}

// ── Commands ─────────────────────────────────────────────────────────────────

async fn list() -> anyhow::Result<()> {
    let specs_dir = find_specs_dir()
        .ok_or_else(|| anyhow::anyhow!("No docs/specs/ directory found"))?;
    let specs = collect_specs(&specs_dir).await?;

    println!("{} Behavioral Specs", "\u{2b21}".cyan());
    println!();

    if specs.is_empty() {
        println!("  {}", "No specs found".dimmed());
        return Ok(());
    }

    let rows: Vec<SpecListRow> = specs
        .iter()
        .map(|(_, s)| SpecListRow {
            feature: s.feature.clone(),
            count: s.specs.len(),
            description: truncate(&s.description, 60),
        })
        .collect();

    println!("{}", HexTable::render(&rows));
    println!();
    println!(
        "  {} spec file(s)  |  {} total scenarios",
        specs.len(),
        specs.iter().map(|(_, s)| s.specs.len()).sum::<usize>()
    );
    Ok(())
}

async fn show(feature: &str, check: bool) -> anyhow::Result<()> {
    let specs_dir = find_specs_dir()
        .ok_or_else(|| anyhow::anyhow!("No docs/specs/ directory found"))?;
    let all = collect_specs(&specs_dir).await?;

    let query = feature.to_lowercase();
    let matches: Vec<&(PathBuf, SpecFile)> = all
        .iter()
        .filter(|(_, s)| s.feature.to_lowercase().contains(&query))
        .collect();

    if matches.is_empty() {
        println!("  {} No spec found matching '{}'", "\u{26a0}".yellow(), feature);
        return Ok(());
    }

    for (path, spec) in &matches {
        // ── Header ──────────────────────────────────────────────────────────
        let cat_count = spec.specs.iter()
            .map(|s| s.category.as_str())
            .filter(|c| !c.is_empty())
            .collect::<std::collections::HashSet<_>>()
            .len();
        let neg_count = spec.specs.iter().filter(|s| s.negative_spec).count();

        println!("{} {}", "\u{2b21}".cyan(), spec.feature.bold());
        if !spec.description.is_empty() {
            println!("  {}", spec.description.dimmed());
        }
        println!(
            "  {} scenarios · {} categories · {} negative",
            spec.specs.len(), cat_count, neg_count
        );
        println!("  {}", path.display().to_string().dimmed());
        println!();

        // ── Scenarios grouped by category ───────────────────────────────────
        let mut categories: Vec<String> = spec.specs.iter()
            .map(|s| s.category.clone())
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .collect();
        categories.sort();

        for cat in &categories {
            println!("  {}", cat.bold().underline());
            for s in spec.specs.iter().filter(|s| &s.category == cat) {
                let sign = if s.negative_spec {
                    format!(" [−]").red().to_string()
                } else {
                    String::new()
                };
                println!("  {} {}{}",
                    s.id.cyan(),
                    s.description.bold(),
                    sign,
                );
                if !s.given.is_empty() {
                    println!("      {} {}", "Given".dimmed(), s.given);
                }
                if !s.when.is_empty() {
                    println!("      {} {}", "When ".dimmed(), s.when);
                }
                if !s.then.is_empty() {
                    println!("      {} {}", "Then ".dimmed(), s.then);
                }
            }
            println!();
        }

        // ── Implementation check ─────────────────────────────────────────────
        if check {
            impl_check(spec).await?;
        }
    }

    Ok(())
}

/// Scan source directories for evidence of each spec category and function.
async fn impl_check(spec: &SpecFile) -> anyhow::Result<()> {
    // Find project root (has docs/specs/)
    let root = find_specs_dir()
        .and_then(|p| p.parent().and_then(|p| p.parent()).map(|p| p.to_path_buf()))
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());

    // Source dirs to search (TypeScript + Rust)
    let src_dirs: Vec<PathBuf> = ["src", "hex-cli/src", "hex-nexus/src", "hex-agent/src"]
        .iter()
        .map(|d| root.join(d))
        .filter(|p| p.is_dir())
        .collect();

    if src_dirs.is_empty() {
        println!("  {} No source directories found to check", "\u{26a0}".yellow());
        return Ok(());
    }

    // Collect unique categories
    let mut categories: Vec<String> = spec.specs.iter()
        .map(|s| s.category.clone())
        .filter(|c| !c.is_empty())
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();
    categories.sort();

    println!("  {} Implementation Check", "\u{2b21}".cyan());
    println!();

    let mut any_missing = false;

    for cat in &categories {
        // Collect function names mentioned in `when` fields for this category
        let fns: Vec<String> = spec.specs.iter()
            .filter(|s| &s.category == cat)
            .flat_map(|s| extract_fn_names(&s.when))
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .collect();

        // grep for the category name across all src dirs
        let cat_found = grep_any(&src_dirs, cat).await;

        if cat_found {
            print!("  {} {}", "\u{2713}".green().bold(), cat.bold());
        } else {
            print!("  {} {}", "\u{2717}".red().bold(), cat.bold());
            any_missing = true;
        }

        // Check each function
        let mut missing_fns: Vec<&str> = Vec::new();
        let mut found_fns: Vec<&str> = Vec::new();
        for f in &fns {
            if grep_any(&src_dirs, f).await {
                found_fns.push(f.as_str());
            } else {
                missing_fns.push(f.as_str());
                any_missing = true;
            }
        }

        if !found_fns.is_empty() || !missing_fns.is_empty() {
            let found_str = found_fns.iter().map(|f| f.green().to_string()).collect::<Vec<_>>().join(", ");
            let miss_str = missing_fns.iter().map(|f| f.red().to_string()).collect::<Vec<_>>().join(", ");
            let parts: Vec<String> = [found_str, miss_str].iter().filter(|s| !s.is_empty()).cloned().collect();
            print!("  →  {}", parts.join("  "));
        }
        println!();
    }

    println!();
    if any_missing {
        println!("  {} Implementation gaps detected — spec is partially unimplemented", "\u{26a0}".yellow().bold());
    } else {
        println!("  {} All categories and functions found in source", "\u{2713}".green().bold());
    }
    println!();

    Ok(())
}

/// Returns true if `term` appears in any file under `dirs`.
async fn grep_any(dirs: &[PathBuf], term: &str) -> bool {
    for dir in dirs {
        let result = tokio::process::Command::new("grep")
            .args(["-r", "-l", "--include=*.ts", "--include=*.rs", "-m", "1", term])
            .arg(dir)
            .output()
            .await;
        if let Ok(out) = result {
            if !out.stdout.is_empty() {
                return true;
            }
        }
    }
    false
}

/// Extract likely function/method names from a `when` clause text.
/// Looks for `identifier(` patterns, filters out short/common words.
fn extract_fn_names(text: &str) -> Vec<String> {
    const SKIP: &[&str] = &[
        "when", "then", "given", "with", "from", "that", "this",
        "true", "false", "null", "void", "call", "called",
    ];
    let mut names = Vec::new();
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '(' && i > 0 {
            // Walk back over identifier chars
            let mut j = i;
            while j > 0 {
                let c = chars[j - 1];
                if c.is_alphanumeric() || c == '_' {
                    j -= 1;
                } else {
                    break;
                }
            }
            let name: String = chars[j..i].iter().collect();
            if name.len() > 4
                && name.chars().next().map(|c| c.is_alphabetic()).unwrap_or(false)
                && !SKIP.contains(&name.to_lowercase().as_str())
            {
                names.push(name);
            }
        }
        i += 1;
    }
    names.sort();
    names.dedup();
    names
}

async fn workplan(feature: &str) -> anyhow::Result<()> {
    let workplans_dir = find_workplans_dir()
        .ok_or_else(|| anyhow::anyhow!("No docs/workplans/ directory found"))?;
    let all_wps = collect_workplans(&workplans_dir).await?;

    let query = feature.to_lowercase();

    let rows: Vec<WorkplanLinkRow> = all_wps
        .iter()
        .filter(|(_, wp)| {
            if let Some(val) = &wp.specs {
                if let Some(p) = workplan_specs_path(val) {
                    return p.to_lowercase().contains(&query);
                }
            }
            false
        })
        .map(|(_, wp)| WorkplanLinkRow {
            id: wp.id.clone(),
            adr: if wp.adr.is_empty() { "\u{2014}".to_string() } else { wp.adr.clone() },
            status: status_badge(if wp.status.is_empty() { "unknown" } else { &wp.status }),
            title: truncate(&wp.title, 55),
        })
        .collect();

    println!("{} Workplans linked to spec '{}'", "\u{2b21}".cyan(), feature.bold());
    println!();

    if rows.is_empty() {
        println!("  {}", "No workplan references this spec".dimmed());
    } else {
        println!("{}", HexTable::render(&rows));
        println!();
        println!("  {} workplan(s)", rows.len());
    }

    Ok(())
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workplan_specs_path_string() {
        let v = serde_json::Value::String("docs/specs/foo.json".to_string());
        assert_eq!(workplan_specs_path(&v), Some("docs/specs/foo.json".to_string()));
    }

    #[test]
    fn workplan_specs_path_non_string() {
        let v = serde_json::json!(["S01", "S02"]);
        assert_eq!(workplan_specs_path(&v), None);
    }
}

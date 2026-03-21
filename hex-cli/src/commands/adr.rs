use std::path::{Path, PathBuf};

use clap::Subcommand;
use colored::Colorize;

#[derive(Subcommand)]
pub enum AdrAction {
    /// List all ADRs with status
    List,
    /// Show ADR lifecycle summary
    Status,
    /// Search ADRs by keyword
    Search {
        /// Search query
        query: String,
    },
    /// Detect stale/abandoned ADRs
    Abandoned,
    /// Review ADRs for consistency issues (ADR-041)
    Review {
        /// Specific ADR to review (e.g. ADR-040). Omit for all.
        adr_id: Option<String>,
        /// Exit non-zero if any WARNING+ findings (for CI)
        #[arg(long)]
        strict: bool,
    },
}

pub async fn run(action: AdrAction) -> anyhow::Result<()> {
    match action {
        AdrAction::List => list().await,
        AdrAction::Status => status().await,
        AdrAction::Search { query } => search(&query).await,
        AdrAction::Abandoned => abandoned().await,
        AdrAction::Review { adr_id, strict } => super::adr_review::run(adr_id, strict).await,
    }
}

/// Discover the ADR directory, searching from the current directory upward.
fn find_adr_dir() -> Option<PathBuf> {
    let cwd = std::env::current_dir().ok()?;
    let mut dir = cwd.as_path();
    loop {
        let candidate = dir.join("docs").join("adrs");
        if candidate.is_dir() {
            return Some(candidate);
        }
        dir = dir.parent()?;
    }
}

/// Parse the status from an ADR markdown file's frontmatter or first heading.
fn parse_adr_status(content: &str) -> &str {
    // Look for "status: <value>" in YAML frontmatter
    for line in content.lines() {
        let trimmed = line.trim().to_lowercase();
        if trimmed.starts_with("status:") {
            let val = line.trim()["status:".len()..].trim();
            // Return a static str approximation
            return match val.to_lowercase().as_str() {
                s if s.contains("proposed") => "proposed",
                s if s.contains("accepted") => "accepted",
                s if s.contains("deprecated") => "deprecated",
                s if s.contains("abandoned") => "abandoned",
                s if s.contains("superseded") => "superseded",
                _ => "unknown",
            };
        }
        // Stop at end of frontmatter
        if trimmed == "---" && content.starts_with("---") && !trimmed.is_empty() {
            // We might be at the closing ---
            // Continue only within frontmatter
        }
    }
    "unknown"
}

/// Collect all ADR files from the directory.
async fn collect_adrs(dir: &Path) -> anyhow::Result<Vec<(PathBuf, String)>> {
    let mut adrs = Vec::new();
    let mut entries = tokio::fs::read_dir(dir).await?;
    while let Some(entry) = entries.next_entry().await? {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("md") {
            let content = tokio::fs::read_to_string(&path).await?;
            adrs.push((path, content));
        }
    }
    adrs.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(adrs)
}

/// Parse the `Enforced-By` field from an ADR markdown.
///
/// Looks for a line starting with `## Enforced-By:` or a frontmatter field
/// `enforced-by:`. Returns Some(description) if found, None otherwise.
fn parse_enforced_by(content: &str) -> Option<String> {
    for line in content.lines() {
        let trimmed = line.trim();
        // Check for heading style: ## Enforced-By: <tool>
        if let Some(rest) = trimmed.strip_prefix("## Enforced-By:") {
            let val = rest.trim();
            if !val.is_empty() {
                return Some(val.to_string());
            }
        }
        // Check for frontmatter style: enforced-by: <tool>
        let lower = trimmed.to_lowercase();
        if lower.starts_with("enforced-by:") {
            let val = trimmed["enforced-by:".len()..].trim();
            if !val.is_empty() {
                return Some(val.to_string());
            }
        }
    }
    None
}

/// Extract the title from an ADR file (first # heading or filename).
fn extract_title(path: &Path, content: &str) -> String {
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("# ") {
            return trimmed[2..].to_string();
        }
    }
    // Fallback to filename
    path.file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("untitled")
        .to_string()
}

async fn list() -> anyhow::Result<()> {
    let adr_dir = find_adr_dir().ok_or_else(|| anyhow::anyhow!("No docs/adrs/ directory found"))?;
    let adrs = collect_adrs(&adr_dir).await?;

    if adrs.is_empty() {
        println!("{} No ADRs found in {}", "\u{2b21}".dimmed(), adr_dir.display());
        return Ok(());
    }

    println!("{} Architecture Decision Records", "\u{2b21}".cyan());
    println!();

    // Header
    println!(
        "  {:<8} {:<12} {:<14} {}",
        "ID".bold(),
        "Status".bold(),
        "Enforcement".bold(),
        "Title".bold()
    );
    println!("  {}", "\u{2500}".repeat(76).dimmed());

    for (path, content) in &adrs {
        let filename = path.file_stem().and_then(|s| s.to_str()).unwrap_or("???");
        let id = filename
            .split('-')
            .next()
            .unwrap_or(filename);
        let status = parse_adr_status(content);
        let title = extract_title(path, content);
        let enforced = parse_enforced_by(content);

        let status_colored = match status {
            "accepted" => status.green().to_string(),
            "proposed" => status.yellow().to_string(),
            "deprecated" => status.red().to_string(),
            "abandoned" => status.red().dimmed().to_string(),
            "superseded" => status.blue().to_string(),
            _ => status.dimmed().to_string(),
        };

        let enforcement_display = match &enforced {
            Some(_) => "\u{2713} enforced".green().to_string(),
            None => "\u{2014} honor system".dimmed().to_string(),
        };

        println!("  {:<8} {:<21} {:<23} {}", id, status_colored, enforcement_display, title);
    }

    println!();
    println!("  {} ADR(s) total", adrs.len());
    Ok(())
}

async fn status() -> anyhow::Result<()> {
    let adr_dir = find_adr_dir().ok_or_else(|| anyhow::anyhow!("No docs/adrs/ directory found"))?;
    let adrs = collect_adrs(&adr_dir).await?;

    let mut counts: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    for (_, content) in &adrs {
        let s = parse_adr_status(content);
        *counts.entry(s).or_insert(0) += 1;
    }

    println!("{} ADR Lifecycle Summary", "\u{2b21}".cyan());
    println!();

    let statuses = ["proposed", "accepted", "deprecated", "superseded", "abandoned", "unknown"];
    for s in &statuses {
        if let Some(&count) = counts.get(s) {
            let colored_status = match *s {
                "accepted" => s.green().to_string(),
                "proposed" => s.yellow().to_string(),
                "deprecated" => s.red().to_string(),
                "abandoned" => s.red().dimmed().to_string(),
                "superseded" => s.blue().to_string(),
                _ => s.dimmed().to_string(),
            };
            println!("  {:<21} {}", colored_status, count);
        }
    }

    println!();
    println!("  {} total", adrs.len());
    Ok(())
}

async fn search(query: &str) -> anyhow::Result<()> {
    let adr_dir = find_adr_dir().ok_or_else(|| anyhow::anyhow!("No docs/adrs/ directory found"))?;
    let adrs = collect_adrs(&adr_dir).await?;

    let query_lower = query.to_lowercase();
    let mut matches = Vec::new();

    for (path, content) in &adrs {
        if content.to_lowercase().contains(&query_lower) {
            let title = extract_title(path, content);
            let status = parse_adr_status(content);

            // Find matching lines for context
            let mut context_lines = Vec::new();
            for line in content.lines() {
                if line.to_lowercase().contains(&query_lower) {
                    context_lines.push(line.trim().to_string());
                    if context_lines.len() >= 3 {
                        break;
                    }
                }
            }

            matches.push((path, title, status, context_lines));
        }
    }

    println!(
        "{} Search results for '{}'",
        "\u{2b21}".cyan(),
        query.bold()
    );
    println!();

    if matches.is_empty() {
        println!("  {}", "No matches found".dimmed());
    } else {
        for (path, title, status, context) in &matches {
            let filename = path.file_name().and_then(|s| s.to_str()).unwrap_or("???");
            println!("  {} [{}]", title.bold(), status);
            println!("  {}", filename.dimmed());
            for line in context {
                println!("    {}", line.dimmed());
            }
            println!();
        }
        println!("  {} match(es)", matches.len());
    }

    Ok(())
}

async fn abandoned() -> anyhow::Result<()> {
    let adr_dir = find_adr_dir().ok_or_else(|| anyhow::anyhow!("No docs/adrs/ directory found"))?;
    let adrs = collect_adrs(&adr_dir).await?;

    println!("{} Stale/Abandoned ADR Detection", "\u{2b21}".cyan());
    println!();

    let mut found = 0;
    for (path, content) in &adrs {
        let status = parse_adr_status(content);
        let title = extract_title(path, content);

        // Flag ADRs that are proposed but possibly stale, or explicitly abandoned
        let is_stale = status == "proposed" || status == "abandoned";
        if is_stale {
            let indicator = if status == "abandoned" {
                "\u{2717}".red()
            } else {
                "?".yellow()
            };
            println!("  {} {} [{}]", indicator, title, status);
            found += 1;
        }
    }

    if found == 0 {
        println!("  {}", "No abandoned or stale ADRs found".green());
    } else {
        println!();
        println!("  {} ADR(s) need attention", found);
    }

    Ok(())
}

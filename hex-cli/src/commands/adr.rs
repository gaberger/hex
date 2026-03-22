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
    /// Show the ADR schema, template, and next available number
    Schema,
}

pub async fn run(action: AdrAction) -> anyhow::Result<()> {
    match action {
        AdrAction::List => list().await,
        AdrAction::Status => status().await,
        AdrAction::Search { query } => search(&query).await,
        AdrAction::Abandoned => abandoned().await,
        AdrAction::Review { adr_id, strict } => super::adr_review::run(adr_id, strict).await,
        AdrAction::Schema => schema().await,
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

async fn schema() -> anyhow::Result<()> {
    let adr_dir = find_adr_dir().ok_or_else(|| anyhow::anyhow!("No docs/adrs/ directory found"))?;

    // Determine next available number by scanning existing ADRs
    let next_number = find_next_adr_number(&adr_dir).await;

    // Try to reserve in SpacetimeDB via nexus
    let reserved = reserve_adr_number(next_number).await;
    let number_source = if reserved { "reserved in SpacetimeDB" } else { "from filesystem scan" };

    println!("{} ADR Schema (for inference engines)", "\u{2b21}".cyan());
    println!();
    println!("  {:<20} {}", "Next number:".bold(), format!("ADR-{:03}", next_number).green());
    println!("  {:<20} {}", "Source:".bold(), number_source.dimmed());
    println!("  {:<20} {}", "Directory:".bold(), adr_dir.display());
    println!("  {:<20} {}", "Filename pattern:".bold(), "ADR-{NNN}-{kebab-slug}.md");
    println!();

    println!("{}", "── Valid statuses ──".bold());
    println!("  Proposed | Accepted | Deprecated | Superseded | Abandoned");
    println!();

    println!("{}", "── Required sections ──".bold());
    println!("  # ADR-{{NNN}}: {{Title}}");
    println!("  **Status:** {{status}}");
    println!("  **Date:** {{YYYY-MM-DD}}");
    println!("  **Drivers:** {{what triggered this}}");
    println!("  ## Context");
    println!("  ## Decision");
    println!("  ## Consequences");
    println!("  ## Implementation");
    println!("  ## References");
    println!();

    println!("{}", "── Template ──".bold());
    // Read and display the template
    let template_path = adr_dir.join("TEMPLATE.md");
    if template_path.exists() {
        let template = tokio::fs::read_to_string(&template_path).await?;
        // Replace the placeholder number with the actual next number
        let filled = template.replace("{NNN}", &format!("{:03}", next_number));
        println!("{}", filled);
    } else {
        println!("  {} TEMPLATE.md not found", "\u{26a0}".yellow());
    }

    // Output machine-readable JSON for inference engines
    println!("{}", "── Machine-readable (JSON) ──".bold());
    let schema_json = serde_json::json!({
        "next_number": next_number,
        "number_source": number_source,
        "directory": adr_dir.to_string_lossy(),
        "filename_pattern": "ADR-{NNN}-{kebab-slug}.md",
        "valid_statuses": ["Proposed", "Accepted", "Deprecated", "Superseded", "Abandoned"],
        "required_sections": ["Context", "Decision", "Consequences", "Implementation", "References"],
        "frontmatter_fields": {
            "Status": "required — one of valid_statuses",
            "Date": "required — YYYY-MM-DD",
            "Drivers": "required — what triggered this decision",
            "Supersedes": "optional — ADR-NNN if replacing an earlier decision"
        }
    });
    println!("{}", serde_json::to_string_pretty(&schema_json)?);

    Ok(())
}

/// Find the next available ADR number by scanning existing files.
async fn find_next_adr_number(adr_dir: &Path) -> u32 {
    let mut max_num: u32 = 0;
    if let Ok(mut entries) = tokio::fs::read_dir(adr_dir).await {
        while let Ok(Some(entry)) = entries.next_entry().await {
            let name = entry.file_name().to_string_lossy().to_string();
            // Match ADR-NNN or adr-NNN patterns
            if let Some(rest) = name.strip_prefix("ADR-").or_else(|| name.strip_prefix("adr-")) {
                if let Some(num_str) = rest.split('-').next() {
                    if let Ok(num) = num_str.parse::<u32>() {
                        if num > max_num {
                            max_num = num;
                        }
                    }
                }
            }
        }
    }
    max_num + 1
}

/// Try to reserve the next ADR number in SpacetimeDB via nexus.
/// Returns true if reservation succeeded, false if nexus unavailable.
async fn reserve_adr_number(number: u32) -> bool {
    let nexus = crate::nexus_client::NexusClient::from_env();
    match nexus.post(
        "/api/adr/reserve",
        &serde_json::json!({ "number": number }),
    ).await {
        Ok(_) => true,
        Err(_) => false,
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_status_accepted() {
        assert_eq!(parse_adr_status("---\nstatus: Accepted\n---\n"), "accepted");
    }

    #[test]
    fn parse_status_proposed() {
        assert_eq!(parse_adr_status("---\nstatus: Proposed\n---\n"), "proposed");
    }

    #[test]
    fn parse_status_missing() {
        assert_eq!(parse_adr_status("# ADR-001: No status here\n\nJust text.\n"), "unknown");
    }

    #[test]
    fn parse_status_case_insensitive() {
        assert_eq!(parse_adr_status("---\nstatus: ACCEPTED\n---\n"), "accepted");
    }

    #[test]
    fn extract_title_from_heading() {
        let path = std::path::Path::new("ADR-001-test.md");
        assert_eq!(extract_title(path, "# ADR-001: My Title\n"), "ADR-001: My Title");
    }

    #[test]
    fn extract_title_fallback_to_filename() {
        let path = std::path::Path::new("ADR-001-test.md");
        assert_eq!(extract_title(path, "No heading here\n"), "ADR-001-test");
    }

    #[test]
    fn parse_enforced_by_heading() {
        let content = "# ADR\n\n## Enforced-By: hex analyze\n";
        assert_eq!(parse_enforced_by(content), Some("hex analyze".to_string()));
    }

    #[test]
    fn parse_enforced_by_missing() {
        assert_eq!(parse_enforced_by("# ADR\n\nNo enforcement.\n"), None);
    }
}

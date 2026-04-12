use std::path::{Path, PathBuf};

use clap::Subcommand;
use colored::Colorize;
use tabled::Tabled;

use crate::fmt::{status_badge, truncate, HexTable};
use super::spec::{find_specs_dir, find_workplans_dir, collect_workplans, workplan_specs_path};

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
    /// Show behavioral specs linked to an ADR via workplans
    Specs {
        /// ADR identifier (e.g. ADR-2603240130 or partial match like 2603240130)
        adr_id: String,
    },
}

pub async fn run(action: AdrAction) -> anyhow::Result<()> {
    match action {
        AdrAction::List => list().await,
        AdrAction::Status => status().await,
        AdrAction::Search { query } => search(&query).await,
        AdrAction::Abandoned => abandoned().await,
        AdrAction::Review { adr_id, strict } => super::adr_review::run(adr_id, strict).await,
        AdrAction::Schema => schema().await,
        AdrAction::Specs { adr_id } => specs_for_adr(&adr_id).await,
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

/// Parse the status from an ADR markdown file.
///
/// Handles three formats:
///   - YAML frontmatter: `status: Accepted`
///   - Bold markdown:    `**Status:** Accepted`
///   - Heading form:     `## Status\nAccepted` (value on next non-empty line)
///
/// Strict-by-design rejections (verified by tests):
///   - `**Status**: Accepted` (colon outside bold)
///   - `- **Status**: Accepted` (bullet-prefixed)
fn parse_adr_status(content: &str) -> &str {
    let lines: Vec<&str> = content.lines().collect();
    let mut i = 0;
    while i < lines.len() {
        let trimmed = lines[i].trim();
        let lower = trimmed.to_lowercase();

        // Extract the value via one of the three accepted formats.
        let val: String = if lower.starts_with("**status:**") {
            // **Status:** Accepted
            trimmed["**Status:**".len()..].trim().to_string()
        } else if lower.starts_with("status:") && !lower.starts_with("status_") {
            // status: Accepted (YAML frontmatter)
            trimmed["status:".len()..].trim().to_string()
        } else if lower == "## status" || lower == "## status:" {
            // ## Status (heading) — value is on the next non-empty line.
            let mut j = i + 1;
            while j < lines.len() && lines[j].trim().is_empty() {
                j += 1;
            }
            if j >= lines.len() {
                i += 1;
                continue;
            }
            // Strip surrounding bold markers, e.g. "**Accepted** | Open" → "Accepted | Open"
            lines[j].trim().trim_matches('*').trim().to_string()
        } else {
            i += 1;
            continue;
        };

        return match val.to_lowercase().as_str() {
            s if s.contains("proposed") => "proposed",
            s if s.contains("accepted") => "accepted",
            s if s.contains("deprecated") => "deprecated",
            s if s.contains("abandoned") => "abandoned",
            s if s.contains("superseded") => "superseded",
            _ => "unknown",
        };
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
            // Only include files that start with "ADR-" (skip TEMPLATE.md, README.md, etc.)
            let fname = path.file_name().and_then(|f| f.to_str()).unwrap_or("");
            if !fname.starts_with("ADR-") {
                continue;
            }
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

/// Extract the ADR ID from a filename stem, e.g. "ADR-059-foo" → "ADR-059",
/// "ADR-2603221500-foo" → "ADR-2603221500".
fn extract_adr_id(filename: &str) -> String {
    // Match "ADR-" followed by digits
    if let Some(rest) = filename.strip_prefix("ADR-").or_else(|| filename.strip_prefix("adr-")) {
        // Take all leading digits
        let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
        if !digits.is_empty() {
            return format!("ADR-{}", digits);
        }
    }
    filename.to_string()
}

/// Extract the title from an ADR file (first # heading or filename).
fn extract_title(path: &Path, content: &str) -> String {
    for line in content.lines() {
        let trimmed = line.trim();
        if let Some(title) = trimmed.strip_prefix("# ") {
            return title.to_string();
        }
    }
    // Fallback to filename
    path.file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("untitled")
        .to_string()
}

// ── Tabled row structs ──────────────────────────────────────────────────

#[derive(Tabled)]
struct AdrListRow {
    #[tabled(rename = "ID")]
    id: String,
    #[tabled(rename = "Status")]
    status: String,
    #[tabled(rename = "Enforcement")]
    enforcement: String,
    #[tabled(rename = "Title")]
    title: String,
}

#[derive(Tabled)]
struct AdrStatusRow {
    #[tabled(rename = "Status")]
    status: String,
    #[tabled(rename = "Count")]
    count: usize,
}

#[derive(Tabled)]
struct AdrSearchRow {
    #[tabled(rename = "ID")]
    id: String,
    #[tabled(rename = "Status")]
    status: String,
    #[tabled(rename = "Title")]
    title: String,
    #[tabled(rename = "Context")]
    context: String,
}

#[derive(Tabled)]
struct AdrAbandonedRow {
    #[tabled(rename = "")]
    indicator: String,
    #[tabled(rename = "Title")]
    title: String,
    #[tabled(rename = "Status")]
    status: String,
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

    let rows: Vec<AdrListRow> = adrs
        .iter()
        .map(|(path, content)| {
            let filename = path.file_stem().and_then(|s| s.to_str()).unwrap_or("???");
            let id = extract_adr_id(filename);
            let status = parse_adr_status(content);
            let title = extract_title(path, content);
            let enforced = parse_enforced_by(content);

            let enforcement = match &enforced {
                Some(_) => "\u{2713} enforced".green().to_string(),
                None => "\u{2014} honor system".dimmed().to_string(),
            };

            AdrListRow {
                id,
                status: status_badge(status),
                enforcement,
                title: truncate(&title, 60),
            }
        })
        .collect();

    println!("{}", HexTable::render(&rows));
    println!();
    println!("  {} ADR(s) total", adrs.len());

    // Warn about any ADRs with unparseable status — likely wrong frontmatter format
    let unknown: Vec<&str> = adrs
        .iter()
        .filter(|(_, content)| parse_adr_status(content) == "unknown")
        .filter_map(|(path, _)| path.file_name()?.to_str())
        .collect();
    if !unknown.is_empty() {
        println!();
        println!(
            "  {} {} ADR(s) have unparseable status — frontmatter must use `**Status:** <value>`:",
            "\u{26a0}".yellow(),
            unknown.len()
        );
        for name in &unknown {
            println!("    {}", name.yellow());
        }
    }

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
    let rows: Vec<AdrStatusRow> = statuses
        .iter()
        .filter_map(|s| {
            counts.get(s).map(|&count| AdrStatusRow {
                status: status_badge(s),
                count,
            })
        })
        .collect();

    println!("{}", HexTable::compact(&rows));
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
        let rows: Vec<AdrSearchRow> = matches
            .iter()
            .map(|(path, title, status, context)| {
                let filename = path.file_stem().and_then(|s| s.to_str()).unwrap_or("???");
                let id = extract_adr_id(filename);
                AdrSearchRow {
                    id,
                    status: status_badge(status),
                    title: truncate(title, 50),
                    context: truncate(&context.join(" | "), 60),
                }
            })
            .collect();

        println!("{}", HexTable::render(&rows));
        println!();
        println!("  {} match(es)", matches.len());
    }

    Ok(())
}

async fn schema() -> anyhow::Result<()> {
    let adr_dir = find_adr_dir().ok_or_else(|| anyhow::anyhow!("No docs/adrs/ directory found"))?;

    // Generate timestamp-based ID (YYMMDDHHMM) — no reservation needed
    let timestamp_id = generate_timestamp_adr_id();
    let now = chrono::Local::now();
    let human_readable = now.format("%Y-%m-%d %H:%M").to_string();

    println!("{} ADR Schema (for inference engines)", "\u{2b21}".cyan());
    println!();
    println!("  {:<20} {}", "Next ID:".bold(), format!("ADR-{}", timestamp_id).green());
    println!("  {:<20} {}", "Readable:".bold(), human_readable.dimmed());
    println!("  {:<20} {}", "Format:".bold(), "YYMMDDHHMM (timestamp, no reservation needed)".dimmed());
    println!("  {:<20} {}", "Directory:".bold(), adr_dir.display());
    println!("  {:<20} ADR-{{YYMMDDHHMM}}-{{kebab-slug}}.md", "Filename pattern:".bold());
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
        let filled = template.replace("{YYMMDDHHMM}", &timestamp_id)
            .replace("{NNN}", &timestamp_id);
        println!("{}", filled);
    } else {
        println!("  {} TEMPLATE.md not found", "\u{26a0}".yellow());
    }

    // Output machine-readable JSON for inference engines
    println!("{}", "── Machine-readable (JSON) ──".bold());
    let schema_json = serde_json::json!({
        "next_id": format!("ADR-{}", timestamp_id),
        "id_format": "YYMMDDHHMM",
        "id_readable": human_readable,
        "directory": adr_dir.to_string_lossy(),
        "filename_pattern": "ADR-{YYMMDDHHMM}-{kebab-slug}.md",
        "valid_statuses": ["Proposed", "Accepted", "Deprecated", "Superseded", "Abandoned"],
        "required_sections": ["Context", "Decision", "Consequences", "Implementation", "References"],
        "frontmatter_fields": {
            "Status": "required — one of valid_statuses",
            "Date": "required — YYYY-MM-DD",
            "Drivers": "required — what triggered this decision",
            "Supersedes": "optional — ADR-YYMMDDHHMM if replacing an earlier decision"
        }
    });
    println!("{}", serde_json::to_string_pretty(&schema_json)?);

    Ok(())
}

/// Generate a timestamp-based ADR ID in YYMMDDHHMM format (ADR-2603221500).
/// This eliminates race conditions from sequential max+1 numbering.
fn generate_timestamp_adr_id() -> String {
    let now = chrono::Local::now();
    now.format("%y%m%d%H%M").to_string()
}

async fn abandoned() -> anyhow::Result<()> {
    let adr_dir = find_adr_dir().ok_or_else(|| anyhow::anyhow!("No docs/adrs/ directory found"))?;
    let adrs = collect_adrs(&adr_dir).await?;

    println!("{} Stale/Abandoned ADR Detection", "\u{2b21}".cyan());
    println!();

    let rows: Vec<AdrAbandonedRow> = adrs
        .iter()
        .filter_map(|(path, content)| {
            let status = parse_adr_status(content);
            let title = extract_title(path, content);

            let is_stale = status == "proposed" || status == "abandoned";
            if is_stale {
                let indicator = if status == "abandoned" {
                    "\u{2717}".red().to_string()
                } else {
                    "?".yellow().to_string()
                };
                Some(AdrAbandonedRow {
                    indicator,
                    title: truncate(&title, 60),
                    status: status_badge(status),
                })
            } else {
                None
            }
        })
        .collect();

    if rows.is_empty() {
        println!("  {}", "No abandoned or stale ADRs found".green());
    } else {
        println!("{}", HexTable::compact(&rows));
        println!();
        println!("  {} ADR(s) need attention", rows.len());
    }

    Ok(())
}

// ── Spec-linkage types ───────────────────────────────────────────────────────

#[derive(serde::Deserialize)]
struct SpecFile {
    feature: String,
    #[serde(default)]
    description: String,
    specs: Vec<SpecScenario>,
}

#[derive(serde::Deserialize)]
struct SpecScenario {
    id: String,
    #[serde(default)]
    category: String,
    description: String,
    #[serde(default)]
    negative_spec: bool,
}

#[derive(Tabled)]
struct SpecRow {
    #[tabled(rename = "ID")]
    id: String,
    #[tabled(rename = "Cat")]
    category: String,
    #[tabled(rename = "Neg")]
    neg: String,
    #[tabled(rename = "Description")]
    description: String,
}

/// `hex adr specs <adr-id>` — find specs linked to an ADR through workplans.
async fn specs_for_adr(adr_id: &str) -> anyhow::Result<()> {
    let workplans_dir = find_workplans_dir()
        .ok_or_else(|| anyhow::anyhow!("No docs/workplans/ directory found"))?;
    let all_wps = collect_workplans(&workplans_dir).await?;

    let query = adr_id.to_uppercase();

    // Find workplans that reference this ADR
    let linked: Vec<_> = all_wps
        .iter()
        .filter(|(_, wp)| wp.adr.to_uppercase().contains(&query))
        .collect();

    println!("{} Specs linked to {}", "\u{2b21}".cyan(), adr_id.bold());
    println!();

    if linked.is_empty() {
        println!(
            "  {} No workplan references ADR '{}'",
            "\u{26a0}".yellow(),
            adr_id
        );
        println!();
        println!(
            "  {} Workplans link specs to ADRs via the `\"adr\"` field in docs/workplans/*.json",
            "\u{2139}".dimmed()
        );
        return Ok(());
    }

    // Find project root for resolving relative spec paths
    let project_root = workplans_dir
        .parent()  // docs/
        .and_then(|p| p.parent())  // project root
        .map(|p| p.to_path_buf());

    for (wp_path, wp) in &linked {
        let wp_name = wp_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or(&wp.id);

        println!(
            "  {} workplan: {}",
            "\u{25b6}".green(),
            wp_name.bold()
        );
        if !wp.title.is_empty() {
            println!("    {}", wp.title.dimmed());
        }

        // Resolve and load the linked spec file
        let spec_path_val = match &wp.specs {
            Some(v) => v.clone(),
            None => {
                println!("    {} (no specs field in workplan)", "\u{2014}".dimmed());
                println!();
                continue;
            }
        };

        let spec_rel = match workplan_specs_path(&spec_path_val) {
            Some(p) => p,
            None => {
                println!("    {} (specs field is not a path string)", "\u{2014}".dimmed());
                println!();
                continue;
            }
        };

        println!("    spec: {}", spec_rel.dimmed());
        println!();

        // Try to load the spec file
        let spec_abs = project_root
            .as_ref()
            .map(|root| root.join(&spec_rel))
            .filter(|p| p.exists());

        // Also try find_specs_dir() + filename
        let spec_abs = spec_abs.or_else(|| {
            find_specs_dir().and_then(|d| {
                let fname = Path::new(&spec_rel).file_name()?;
                Some(d.join(fname))
            })
        });

        match spec_abs {
            Some(abs) if abs.exists() => {
                let raw = tokio::fs::read_to_string(&abs).await?;
                match serde_json::from_str::<SpecFile>(&raw) {
                    Ok(spec) => {
                        println!(
                            "    {} — {} ({} scenarios)",
                            spec.feature.bold(),
                            spec.description.dimmed(),
                            spec.specs.len()
                        );
                        println!();

                        let rows: Vec<SpecRow> = spec.specs.iter().map(|s| SpecRow {
                            id: s.id.clone(),
                            category: s.category.clone(),
                            neg: if s.negative_spec { "\u{2212}".red().to_string() } else { String::new() },
                            description: truncate(&s.description, 55),
                        }).collect();

                        println!("{}", HexTable::compact(&rows));
                        println!();
                        println!(
                            "    {} Run `hex spec show {}` for Given/When/Then detail",
                            "\u{2139}".dimmed(),
                            spec.feature
                        );
                    }
                    Err(e) => {
                        println!("    {} Failed to parse spec: {}", "\u{2717}".red(), e);
                    }
                }
            }
            _ => {
                println!("    {} Spec file not found at '{}'", "\u{26a0}".yellow(), spec_rel);
            }
        }

        println!();
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
    fn parse_status_bold_markdown() {
        assert_eq!(parse_adr_status("# ADR-001\n\n**Status:** Accepted\n**Date:** 2026-01-01\n"), "accepted");
    }

    #[test]
    fn parse_status_bold_proposed() {
        assert_eq!(parse_adr_status("# ADR\n**Status:** Proposed\n"), "proposed");
    }

    #[test]
    fn parse_status_missing() {
        assert_eq!(parse_adr_status("# ADR-001: No status here\n\nJust text.\n"), "unknown");
    }

    #[test]
    fn parse_status_wrong_format_colon_outside_bold() {
        // **Status**: Accepted  ← colon outside ** — must NOT parse (agent wrote wrong format)
        assert_eq!(parse_adr_status("# ADR-001\n\n**Status**: Accepted\n"), "unknown");
    }

    #[test]
    fn parse_status_bullet_prefix_not_parsed() {
        // - **Status**: Accepted  ← bullet + colon outside — must NOT parse
        assert_eq!(parse_adr_status("# ADR-001\n\n- **Status**: Accepted\n"), "unknown");
    }

    #[test]
    fn parse_status_heading_form_plain() {
        // ## Status\nAccepted — value on the next line
        assert_eq!(parse_adr_status("# ADR-001\n\n## Status\nAccepted\n"), "accepted");
    }

    #[test]
    fn parse_status_heading_form_with_bold_value() {
        // ## Status\n**Accepted** | Open — surrounding bold markers must be stripped
        assert_eq!(
            parse_adr_status("# ADR-001\n\n## Status\n**Accepted** | Open\n"),
            "accepted"
        );
    }

    #[test]
    fn parse_status_heading_form_with_date_suffix() {
        // ## Status\n**Accepted** — 2026-04-10
        assert_eq!(
            parse_adr_status("# ADR-001\n\n## Status\n**Accepted** — 2026-04-10\n"),
            "accepted"
        );
    }

    #[test]
    fn parse_status_heading_form_blank_line_before_value() {
        // ## Status\n\nProposed — blank line between heading and value
        assert_eq!(parse_adr_status("# ADR-001\n\n## Status\n\nProposed\n"), "proposed");
    }

    #[test]
    fn parse_status_heading_form_with_colon() {
        // ## Status: heading with trailing colon
        assert_eq!(parse_adr_status("# ADR-001\n\n## Status:\nAccepted\n"), "accepted");
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

    // ── Timestamp ID tests (ADR-2603221500) ──

    #[test]
    fn extract_adr_id_legacy() {
        assert_eq!(extract_adr_id("ADR-059-canonical-project-identity"), "ADR-059");
    }

    #[test]
    fn extract_adr_id_timestamp() {
        assert_eq!(extract_adr_id("ADR-2603221500-timestamp-adr-numbering"), "ADR-2603221500");
    }

    #[test]
    fn extract_adr_id_case_insensitive() {
        assert_eq!(extract_adr_id("adr-001-foo"), "ADR-001");
    }

    #[test]
    fn extract_adr_id_no_prefix() {
        assert_eq!(extract_adr_id("TEMPLATE"), "TEMPLATE");
    }

    #[test]
    fn generate_timestamp_id_format() {
        let id = generate_timestamp_adr_id();
        // Should be exactly 10 digits (YYMMDDHHMM)
        assert_eq!(id.len(), 10, "Timestamp ID should be 10 digits, got: {}", id);
        assert!(id.chars().all(|c| c.is_ascii_digit()), "Should be all digits: {}", id);
    }

    #[test]
    fn extract_title_timestamp_adr() {
        let path = std::path::Path::new("ADR-2603221500-test.md");
        assert_eq!(
            extract_title(path, "# ADR-2603221500: My Title\n"),
            "ADR-2603221500: My Title"
        );
    }
}

//! `hex assets` — inspect and sync embedded assets baked into the binary.
//!
//! `hex assets list`  — enumerate every file in the rust-embed bundle.
//! `hex assets sync`  — extract skills, agents, hooks, and settings into
//!                      the current project's `.claude/` directory.
//!
//! `sync` works offline — it reads directly from the binary's embedded
//! bundle, with no nexus dependency. This is the canonical way to update
//! a project's Claude Code environment after upgrading the hex binary.

use anyhow::{Context, Result};
use clap::Subcommand;
use colored::Colorize;
use std::fs;
use std::path::{Path, PathBuf};

use crate::assets::Assets;

#[derive(Subcommand)]
pub enum AssetsAction {
    /// List all embedded assets baked into the binary
    List,
    /// Sync embedded assets into the current project's .claude/ directory
    Sync {
        /// Target project directory (defaults to current directory)
        #[arg(default_value = ".")]
        path: String,

        /// Preview what would change without writing files
        #[arg(long)]
        dry_run: bool,

        /// Also update .mcp.json and CLAUDE.md hex section
        #[arg(long, short)]
        force: bool,
    },
}

pub async fn run(action: AssetsAction) -> Result<()> {
    match action {
        AssetsAction::List => list().await,
        AssetsAction::Sync {
            path,
            dry_run,
            force,
        } => sync(&path, dry_run, force).await,
    }
}

// ── hex assets list ──────────────────────────────────────────────────

async fn list() -> Result<()> {
    println!("{} Embedded Assets (ADR-2603221522)", "\u{2b21}".cyan());
    println!();

    let mut total_size: usize = 0;
    let mut count: usize = 0;

    let mut paths: Vec<_> = Assets::iter().collect();
    paths.sort();

    for path in &paths {
        if let Some(file) = Assets::get(path) {
            let size = file.data.len();
            total_size += size;
            count += 1;
            println!("  {} ({} bytes)", path, size);
        }
    }

    println!();
    println!("  {} asset(s), {} bytes total", count, total_size);
    Ok(())
}

// ── hex assets sync ──────────────────────────────────────────────────

/// Asset prefix → target subdirectory under `.claude/`
const SYNC_MAPPINGS: &[(&str, &str)] = &[
    ("skills/", ".claude/skills/"),
    ("agents/hex/hex/", ".claude/agents/hex/hex/"),
    ("hooks/hex/", ".claude/hooks/hex/"),
];

async fn sync(path: &str, dry_run: bool, force: bool) -> Result<()> {
    let target = PathBuf::from(path)
        .canonicalize()
        .unwrap_or_else(|_| PathBuf::from(path));

    // Guard: must be a hex-aware project (or --force)
    let hex_dir = target.join(".hex");
    if !hex_dir.exists() && !force {
        anyhow::bail!(
            "Not a hex project (no .hex/ directory). Run `hex init` first, or use --force."
        );
    }

    let label = if dry_run { "DRY RUN" } else { "Syncing" };
    println!(
        "{} {} assets → {}",
        "\u{2b21}".cyan(),
        label,
        target.display().to_string().bold()
    );
    println!();

    let mut updated: Vec<String> = Vec::new();
    let mut created: Vec<String> = Vec::new();
    let mut unchanged: usize = 0;

    // ── 1. Extract skills, agents, hooks ──────────────────────────
    for &(prefix, dest_subdir) in SYNC_MAPPINGS {
        let dest_dir = target.join(dest_subdir);

        for asset_path in Assets::iter() {
            if let Some(relative) = asset_path.strip_prefix(prefix) {
                if relative.is_empty() {
                    continue;
                }
                // Strip .tmpl extension in output path
                let dest_name = relative.strip_suffix(".tmpl").unwrap_or(relative);
                let dest = dest_dir.join(dest_name);

                if let Some(file) = Assets::get(&asset_path) {
                    let new_content = &file.data;
                    let display_path = format!("{}{}", dest_subdir, dest_name);

                    if dest.exists() {
                        let existing = fs::read(&dest).unwrap_or_default();
                        if existing == new_content.as_ref() {
                            unchanged += 1;
                            continue;
                        }
                        // Content differs — update
                        if !dry_run {
                            if let Some(parent) = dest.parent() {
                                fs::create_dir_all(parent)?;
                            }
                            fs::write(&dest, new_content.as_ref())
                                .with_context(|| format!("Failed to write {}", dest.display()))?;
                        }
                        updated.push(display_path);
                    } else {
                        // New file
                        if !dry_run {
                            if let Some(parent) = dest.parent() {
                                fs::create_dir_all(parent)?;
                            }
                            fs::write(&dest, new_content.as_ref())
                                .with_context(|| format!("Failed to write {}", dest.display()))?;
                        }
                        created.push(display_path);
                    }
                }
            }
        }
    }

    // ── 2. Merge settings.json ────────────────────────────────────
    let settings_changes = sync_settings(&target, dry_run)?;

    // ── 3. Optionally update .mcp.json and CLAUDE.md ──────────────
    let mut extras: Vec<String> = Vec::new();
    if force {
        if let Some(msg) = sync_mcp_json(&target, dry_run)? {
            extras.push(msg);
        }
        if let Some(msg) = sync_claude_md(&target, dry_run)? {
            extras.push(msg);
        }
    }

    // ── Summary ───────────────────────────────────────────────────
    if !created.is_empty() {
        println!("  {} Created:", "＋".green());
        for f in &created {
            println!("    {} {}", "\u{2713}".green(), f);
        }
    }
    if !updated.is_empty() {
        println!("  {} Updated:", "△".yellow());
        for f in &updated {
            println!("    {} {}", "\u{2713}".yellow(), f);
        }
    }
    for line in &settings_changes {
        println!("    {} {}", "\u{2713}".yellow(), line);
    }
    for line in &extras {
        println!("    {} {}", "\u{2713}".yellow(), line);
    }

    let total_changes = created.len() + updated.len() + settings_changes.len() + extras.len();
    println!();
    if total_changes == 0 {
        println!(
            "  {} Everything up to date ({} files checked)",
            "\u{2713}".green(),
            unchanged
        );
    } else if dry_run {
        println!(
            "  {} {} file(s) would change, {} unchanged",
            "\u{2022}".dimmed(),
            total_changes,
            unchanged
        );
        println!(
            "  {} Run without --dry-run to apply",
            "\u{2022}".dimmed()
        );
    } else {
        println!(
            "  {} {} created, {} updated, {} unchanged",
            "\u{2713}".green(),
            created.len(),
            updated.len() + settings_changes.len() + extras.len(),
            unchanged
        );
    }

    Ok(())
}

// ── Settings merge ───────────────────────────────────────────────────
//
// Merges hooks and statusLine from the embedded template into the
// project's .claude/settings.json. Preserves user-customized fields
// (permissions, custom keys). Individual hook entries are merged by
// event matcher so user additions survive.

fn sync_settings(target: &Path, dry_run: bool) -> Result<Vec<String>> {
    let settings_path = target.join(".claude/settings.json");
    let template_str = Assets::get_str("templates/hex-claude-settings.json")
        .context("hex-claude-settings.json not found in embedded assets")?;
    let template: serde_json::Value = serde_json::from_str(&template_str)
        .context("Failed to parse embedded settings template")?;

    let mut changes: Vec<String> = Vec::new();

    // Load existing settings or start fresh
    let mut settings: serde_json::Value = if settings_path.exists() {
        let existing = fs::read_to_string(&settings_path)?;
        serde_json::from_str(&existing).unwrap_or_else(|_| serde_json::json!({}))
    } else {
        changes.push(".claude/settings.json (created)".to_string());
        serde_json::json!({})
    };

    let original = settings.to_string();

    // Merge hooks: replace hex-managed hooks, preserve user-added ones
    if let Some(template_hooks) = template.get("hooks") {
        merge_hooks(&mut settings, template_hooks);
    }

    // Overwrite statusLine (hex-managed)
    if let Some(status_line) = template.get("statusLine") {
        settings["statusLine"] = status_line.clone();
    }

    // Overwrite companyAnnouncements (hex-managed)
    if let Some(announcements) = template.get("companyAnnouncements") {
        settings["companyAnnouncements"] = announcements.clone();
    }

    // Preserve permissions — only set if missing
    if settings.get("permissions").is_none() {
        if let Some(perms) = template.get("permissions") {
            settings["permissions"] = perms.clone();
        }
    }

    let updated = settings.to_string();
    if original != updated {
        if changes.is_empty() {
            changes.push(".claude/settings.json (merged)".to_string());
        }
        if !dry_run {
            let claude_dir = target.join(".claude");
            fs::create_dir_all(&claude_dir)?;
            fs::write(
                &settings_path,
                serde_json::to_string_pretty(&settings)?,
            )
            .context("Failed to write .claude/settings.json")?;
        }
    }

    Ok(changes)
}

/// Merge hook arrays by event matcher key.
///
/// For each hook event type (UserPromptSubmit, SubagentStart, etc.),
/// template hooks whose command starts with "hex " are considered
/// hex-managed. We replace those while preserving any user-added hooks
/// (commands that don't start with "hex ").
fn merge_hooks(settings: &mut serde_json::Value, template_hooks: &serde_json::Value) {
    let template_obj = match template_hooks.as_object() {
        Some(o) => o,
        None => return,
    };

    let settings_hooks = settings
        .as_object_mut()
        .and_then(|o| {
            o.entry("hooks")
                .or_insert_with(|| serde_json::json!({}))
                .as_object_mut()
                .cloned()
        });

    let mut merged = serde_json::Map::new();

    // Start with existing hooks
    if let Some(existing) = &settings_hooks {
        for (key, val) in existing {
            merged.insert(key.clone(), val.clone());
        }
    }

    // For each event type in template, merge hex-managed hooks
    for (event_type, template_entries) in template_obj {
        let template_arr = match template_entries.as_array() {
            Some(a) => a,
            None => continue,
        };

        let existing_arr = merged
            .get(event_type)
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        // Keep user hooks (commands NOT starting with "hex ")
        let user_hooks: Vec<_> = existing_arr
            .iter()
            .filter(|entry| {
                let cmd = entry.get("command").and_then(|c| c.as_str()).unwrap_or("");
                !cmd.starts_with("hex ")
            })
            .cloned()
            .collect();

        // Combine: all template hooks + user hooks
        let mut combined = template_arr.clone();
        combined.extend(user_hooks);

        merged.insert(event_type.clone(), serde_json::Value::Array(combined));
    }

    settings["hooks"] = serde_json::Value::Object(merged);
}

// ── .mcp.json sync (--force only) ────────────────────────────────────

fn sync_mcp_json(target: &Path, dry_run: bool) -> Result<Option<String>> {
    let mcp_path = target.join(".mcp.json");

    let mut mcp: serde_json::Value = if mcp_path.exists() {
        let existing = fs::read_to_string(&mcp_path)?;
        serde_json::from_str(&existing).unwrap_or_else(|_| serde_json::json!({"mcpServers": {}}))
    } else {
        serde_json::json!({"mcpServers": {}})
    };

    let original = mcp.to_string();

    mcp["mcpServers"]["hex"] = serde_json::json!({
        "command": "hex",
        "args": ["mcp"],
        "toolSearch": {
            "type": "tool_search_tool_bm25_20251119",
            "enabled": true
        }
    });

    let updated = mcp.to_string();
    if original == updated {
        return Ok(None);
    }

    if !dry_run {
        fs::write(&mcp_path, serde_json::to_string_pretty(&mcp)?)
            .context("Failed to write .mcp.json")?;
    }

    Ok(Some(".mcp.json (hex server entry updated)".to_string()))
}

// ── CLAUDE.md hex section sync (--force only) ────────────────────────

fn sync_claude_md(target: &Path, dry_run: bool) -> Result<Option<String>> {
    let claude_md_path = target.join("CLAUDE.md");
    if !claude_md_path.exists() {
        return Ok(None);
    }

    let existing = fs::read_to_string(&claude_md_path)?;

    let hex_section = Assets::get_str("templates/claude-md-hex-section.md")
        .context("claude-md-hex-section.md not found in embedded assets")?;

    // Find and replace existing hex section, or append
    let marker_start = "## Hexagonal Architecture Rules";
    if let Some(start_idx) = existing.find(marker_start) {
        // Replace from marker to end of file (hex section is always at the end)
        // Look for the next top-level heading after the hex section to preserve content after it
        let after_marker = &existing[start_idx..];
        let hex_section_end = find_hex_section_end(after_marker);
        let before = &existing[..start_idx];
        let after = &existing[start_idx + hex_section_end..];

        let new_content = format!("{}{}{}", before.trim_end(), "\n\n", hex_section.trim());
        let new_content = if !after.trim().is_empty() {
            format!("{}\n\n{}", new_content, after.trim())
        } else {
            format!("{}\n", new_content)
        };

        if new_content.trim() == existing.trim() {
            return Ok(None);
        }

        if !dry_run {
            fs::write(&claude_md_path, new_content)
                .context("Failed to write CLAUDE.md")?;
        }

        Ok(Some("CLAUDE.md (hex section updated)".to_string()))
    } else {
        // Append hex section
        let new_content = format!("{}\n\n{}\n", existing.trim(), hex_section.trim());
        if !dry_run {
            fs::write(&claude_md_path, new_content)
                .context("Failed to write CLAUDE.md")?;
        }
        Ok(Some("CLAUDE.md (hex section appended)".to_string()))
    }
}

/// Find where the hex section ends in the CLAUDE.md content.
/// The hex section runs from `## Hexagonal Architecture Rules` until
/// either the next `## ` heading that isn't part of the hex template,
/// or end of string.
fn find_hex_section_end(from_marker: &str) -> usize {
    // The hex section template contains known sub-headings.
    // We look for a `## ` that ISN'T one of the known hex sub-headings.
    let known_hex_headings = [
        "## Hexagonal Architecture Rules",
        "## Security",
    ];

    let lines = from_marker.lines();
    let mut pos = 0;
    let mut first = true;

    for line in lines {
        if !first && line.starts_with("## ") {
            let is_known = known_hex_headings.iter().any(|h| line.starts_with(h));
            if !is_known {
                return pos;
            }
        }
        first = false;
        pos += line.len() + 1; // +1 for newline
    }

    from_marker.len()
}

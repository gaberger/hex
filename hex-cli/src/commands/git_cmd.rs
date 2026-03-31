//! Git integration commands.
//!
//! `hex git status|log|diff|branches|cleanup` — delegates to hex-nexus git API
//! (except cleanup which runs git locally).

use clap::Subcommand;
use colored::Colorize;

use crate::nexus_client::NexusClient;

#[derive(Subcommand)]
pub enum GitAction {
    /// Show working tree status (staged, modified, untracked)
    Status,
    /// Show recent commit log
    Log {
        /// Maximum number of commits to show
        #[arg(short = 'n', long, default_value = "20")]
        limit: u32,
    },
    /// Show unstaged diff
    Diff,
    /// List branches
    Branches,
    /// Delete merged and stale branches (dry-run by default)
    Cleanup {
        /// Actually delete branches (default is dry-run)
        #[arg(long)]
        force: bool,
        /// Also delete unmerged stale branches matching known patterns (worktree-agent-*, hex-go/*, hex/uuid-*)
        #[arg(long)]
        all: bool,
    },
}

pub async fn run(action: GitAction) -> anyhow::Result<()> {
    match action {
        GitAction::Status => status().await,
        GitAction::Log { limit } => log(limit).await,
        GitAction::Diff => diff().await,
        GitAction::Branches => branches().await,
        GitAction::Cleanup { force, all } => cleanup(force, all).await,
    }
}

/// Resolve project_id from .hex/project.json in cwd, falling back to directory name.
fn resolve_project_id() -> String {
    let cwd = std::env::current_dir().unwrap_or_default();

    // Try .hex/project.json first
    let project_json = cwd.join(".hex/project.json");
    if let Ok(content) = std::fs::read_to_string(&project_json) {
        if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&content) {
            if let Some(id) = parsed["id"].as_str() {
                if !id.is_empty() {
                    return id.to_string();
                }
            }
        }
    }

    // Fallback: directory name (matches hex-nexus make_project_id)
    cwd.file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown")
        .to_string()
}

async fn status() -> anyhow::Result<()> {
    let nexus = NexusClient::from_env();
    nexus.ensure_running().await?;

    let project_id = resolve_project_id();
    let raw = nexus
        .get(&format!("/api/{}/git/status", project_id))
        .await?;
    let resp = if raw["data"].is_object() { raw["data"].clone() } else { raw };

    println!("{}", "Git Status".bold());
    println!("{}", "─".repeat(60));

    if let Some(branch) = resp["branch"].as_str() {
        println!("  Branch: {}", branch.cyan());
    }

    // Staged files
    if let Some(staged) = resp["staged"].as_array() {
        if !staged.is_empty() {
            println!("\n  {} Staged:", "\u{25cf}".green());
            for f in staged {
                let path = f["path"].as_str().or(f.as_str()).unwrap_or("?");
                let status = f["status"].as_str().unwrap_or("M");
                let indicator = match status {
                    "A" => "+".green(),
                    "D" => "-".red(),
                    _ => "~".yellow(),
                };
                println!("    {} {}", indicator, path);
            }
        }
    }

    // Modified (unstaged)
    if let Some(modified) = resp["modified"].as_array().or(resp["unstaged"].as_array()) {
        if !modified.is_empty() {
            println!("\n  {} Modified:", "\u{25cb}".yellow());
            for f in modified {
                let path = f["path"].as_str().or(f.as_str()).unwrap_or("?");
                println!("    {} {}", "~".yellow(), path);
            }
        }
    }

    // Untracked
    if let Some(untracked) = resp["untracked"].as_array() {
        if !untracked.is_empty() {
            println!("\n  {} Untracked:", "?".dimmed());
            for f in untracked {
                let path = f["path"].as_str().or(f.as_str()).unwrap_or("?");
                println!("    {} {}", "?".dimmed(), path);
            }
        }
    }

    // If clean
    let is_clean = resp["staged"]
        .as_array()
        .map(|a| a.is_empty())
        .unwrap_or(true)
        && resp["modified"]
            .as_array()
            .or(resp["unstaged"].as_array())
            .map(|a| a.is_empty())
            .unwrap_or(true)
        && resp["untracked"]
            .as_array()
            .map(|a| a.is_empty())
            .unwrap_or(true);

    if is_clean {
        println!("  {} Working tree clean", "\u{2713}".green());
    }

    Ok(())
}

async fn log(limit: u32) -> anyhow::Result<()> {
    let nexus = NexusClient::from_env();
    nexus.ensure_running().await?;

    let project_id = resolve_project_id();
    let raw = nexus
        .get(&format!("/api/{}/git/log?limit={}", project_id, limit))
        .await?;
    let resp = if raw["data"].is_object() { raw["data"].clone() } else { raw };

    println!("{}", "Git Log".bold());
    println!("{}", "─".repeat(60));

    if let Some(commits) = resp["commits"].as_array().or(resp.as_array()) {
        if commits.is_empty() {
            println!("  No commits found.");
            return Ok(());
        }
        for c in commits {
            let sha = c["sha"].as_str().or(c["hash"].as_str()).unwrap_or("???????");
            let short = if sha.len() >= 7 { &sha[..7] } else { sha };
            let msg = c["message"]
                .as_str()
                .or(c["subject"].as_str())
                .unwrap_or("")
                .lines()
                .next()
                .unwrap_or("");
            let author = c["author"].as_str().unwrap_or("");
            let date = c["date"].as_str().unwrap_or("");

            println!(
                "  {} {} {}",
                short.yellow(),
                msg,
                if !author.is_empty() {
                    format!("({})", author).dimmed().to_string()
                } else {
                    String::new()
                }
            );
            if !date.is_empty() {
                println!("         {}", date.dimmed());
            }
        }
    } else {
        println!("  No commits found.");
    }

    Ok(())
}

async fn diff() -> anyhow::Result<()> {
    let nexus = NexusClient::from_env();
    nexus.ensure_running().await?;

    let project_id = resolve_project_id();
    let raw = nexus
        .get(&format!("/api/{}/git/diff", project_id))
        .await?;
    let resp = if raw["data"].is_object() { raw["data"].clone() } else { raw };

    println!("{}", "Git Diff".bold());
    println!("{}", "─".repeat(60));

    if let Some(diff_text) = resp["diff"].as_str() {
        if diff_text.is_empty() {
            println!("  No unstaged changes.");
        } else {
            // Colorize diff output
            for line in diff_text.lines() {
                if line.starts_with('+') && !line.starts_with("+++") {
                    println!("{}", line.green());
                } else if line.starts_with('-') && !line.starts_with("---") {
                    println!("{}", line.red());
                } else if line.starts_with("@@") {
                    println!("{}", line.cyan());
                } else if line.starts_with("diff ") {
                    println!("{}", line.bold());
                } else {
                    println!("{}", line);
                }
            }
        }
    } else if let Some(files) = resp["files"].as_array() {
        if files.is_empty() {
            println!("  No unstaged changes.");
        } else {
            for f in files {
                let path = f["path"].as_str().unwrap_or("?");
                let insertions = f["insertions"].as_u64().unwrap_or(0);
                let deletions = f["deletions"].as_u64().unwrap_or(0);
                println!(
                    "  {} {} {}",
                    path,
                    format!("+{}", insertions).green(),
                    format!("-{}", deletions).red()
                );
            }
        }
    } else {
        println!("  No unstaged changes.");
    }

    Ok(())
}

async fn cleanup(force: bool, all: bool) -> anyhow::Result<()> {
    // Collect merged branches (safe to delete with -d)
    let merged_out = std::process::Command::new("git")
        .args(["branch", "--merged", "main"])
        .output()?;
    let merged_raw = String::from_utf8_lossy(&merged_out.stdout);

    // Each line may be prefixed with "* " (current), "+ " (worktree), or "  " (normal)
    let merged: Vec<(String, bool)> = merged_raw
        .lines()
        .map(|l| {
            let in_worktree = l.starts_with('+');
            let name = l.trim_start_matches(['*', '+', ' ']).trim().to_string();
            (name, in_worktree)
        })
        .filter(|(name, _)| !name.is_empty() && name != "main")
        .collect();

    // Collect stale pattern branches (may or may not be merged)
    let all_out = std::process::Command::new("git")
        .args(["branch", "--list"])
        .output()?;
    let all_raw = String::from_utf8_lossy(&all_out.stdout);

    let stale_patterns: &[&str] = &[
        "worktree-agent-",
        "hex-go/",
        "hex/",          // hex/uuid-* swarm branches
    ];

    let stale_unmerged: Vec<(String, bool)> = all_raw
        .lines()
        .map(|l| {
            let in_worktree = l.starts_with('+');
            let name = l.trim_start_matches(['*', '+', ' ']).trim().to_string();
            (name, in_worktree)
        })
        .filter(|(name, _)| {
            if name.is_empty() || name == "main" { return false; }
            stale_patterns.iter().any(|p| name.starts_with(p))
        })
        // Exclude already in merged list
        .filter(|(name, _)| !merged.iter().any(|(m, _)| m == name))
        .collect();

    println!("{}", "Git Branch Cleanup".bold());
    println!("{}", "─".repeat(60));

    if merged.is_empty() && (!all || stale_unmerged.is_empty()) {
        println!("  {} Nothing to clean up.", "\u{2713}".green());
        return Ok(());
    }

    // Show merged branches
    if !merged.is_empty() {
        println!("\n  {} Merged into main (safe to delete):", "\u{25cf}".green());
        for (name, in_worktree) in &merged {
            let wt_note = if *in_worktree { " (checked out in worktree — skipped)".dimmed().to_string() } else { String::new() };
            println!("    {} {}{}", "-".green(), name, wt_note);
        }
    }

    // Show stale unmerged branches
    if all && !stale_unmerged.is_empty() {
        println!("\n  {} Stale pattern branches (unmerged):", "\u{25cb}".yellow());
        for (name, in_worktree) in &stale_unmerged {
            let wt_note = if *in_worktree { " (checked out in worktree — skipped)".dimmed().to_string() } else { String::new() };
            println!("    {} {}{}", "-".yellow(), name, wt_note);
        }
    }

    if !force {
        println!("\n  {} Dry run — pass {} to delete.", "\u{26a0}".yellow(), "--force".bold());
        let deletable_merged = merged.iter().filter(|(_, wt)| !wt).count();
        let deletable_stale = if all { stale_unmerged.iter().filter(|(_, wt)| !wt).count() } else { 0 };
        println!("  Would delete {} merged + {} stale branches.", deletable_merged, deletable_stale);
        return Ok(());
    }

    // Delete merged branches (skip worktree-checked-out ones)
    let mut deleted = 0usize;
    let mut skipped = 0usize;
    for (name, in_worktree) in &merged {
        if *in_worktree {
            skipped += 1;
            continue;
        }
        let result = std::process::Command::new("git")
            .args(["branch", "-d", name])
            .output()?;
        if result.status.success() {
            println!("  {} Deleted (merged): {}", "\u{2713}".green(), name);
            deleted += 1;
        } else {
            let err = String::from_utf8_lossy(&result.stderr);
            println!("  {} Failed to delete {}: {}", "\u{2717}".red(), name, err.trim());
        }
    }

    // Delete stale unmerged branches with -D if --all specified
    if all {
        for (name, in_worktree) in &stale_unmerged {
            if *in_worktree {
                skipped += 1;
                continue;
            }
            let result = std::process::Command::new("git")
                .args(["branch", "-D", name])
                .output()?;
            if result.status.success() {
                println!("  {} Deleted (stale): {}", "\u{2713}".yellow(), name);
                deleted += 1;
            } else {
                let err = String::from_utf8_lossy(&result.stderr);
                println!("  {} Failed to delete {}: {}", "\u{2717}".red(), name, err.trim());
            }
        }
    }

    println!("\n  Deleted {} branch(es), skipped {} (active worktrees).", deleted, skipped);
    Ok(())
}

async fn branches() -> anyhow::Result<()> {
    let nexus = NexusClient::from_env();
    nexus.ensure_running().await?;

    let project_id = resolve_project_id();
    let raw = nexus
        .get(&format!("/api/{}/git/branches", project_id))
        .await?;
    let resp = if raw["data"].is_object() || raw["data"].is_array() { raw["data"].clone() } else { raw };

    println!("{}", "Git Branches".bold());
    println!("{}", "─".repeat(60));

    if let Some(branches) = resp["branches"].as_array().or(resp.as_array()) {
        if branches.is_empty() {
            println!("  No branches found.");
            return Ok(());
        }
        for b in branches {
            let name = b["name"].as_str().unwrap_or("?");
            let is_current = b["current"].as_bool().or(b["head"].as_bool()).unwrap_or(false);
            let indicator = if is_current {
                "\u{25cf}".green().to_string()
            } else {
                "\u{25cb}".dimmed().to_string()
            };
            let display = if is_current {
                name.green().bold().to_string()
            } else {
                name.to_string()
            };
            println!("  {} {}", indicator, display);
        }
    } else {
        println!("  No branches found.");
    }

    Ok(())
}

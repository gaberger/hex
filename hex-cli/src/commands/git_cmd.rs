//! Git integration commands.
//!
//! `hex git status|log|diff|branches` — delegates to hex-nexus git API.

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
}

pub async fn run(action: GitAction) -> anyhow::Result<()> {
    match action {
        GitAction::Status => status().await,
        GitAction::Log { limit } => log(limit).await,
        GitAction::Diff => diff().await,
        GitAction::Branches => branches().await,
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
    let resp = nexus
        .get(&format!("/api/{}/git/status", project_id))
        .await?;

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
    let resp = nexus
        .get(&format!("/api/{}/git/log?limit={}", project_id, limit))
        .await?;

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
    let resp = nexus
        .get(&format!("/api/{}/git/diff", project_id))
        .await?;

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

async fn branches() -> anyhow::Result<()> {
    let nexus = NexusClient::from_env();
    nexus.ensure_running().await?;

    let project_id = resolve_project_id();
    let resp = nexus
        .get(&format!("/api/{}/git/branches", project_id))
        .await?;

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

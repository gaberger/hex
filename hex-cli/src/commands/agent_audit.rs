//! `hex agent worktree-audit` — show all active git worktrees with assigned agent, task, and age.

use colored::Colorize;
use serde_json::Value;
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, SystemTime};
use tabled::Tabled;

use crate::fmt::HexTable;

#[derive(Tabled)]
struct WorktreeRow {
    #[tabled(rename = "Worktree")]
    worktree: String,
    #[tabled(rename = "Branch")]
    branch: String,
    #[tabled(rename = "Agent")]
    agent: String,
    #[tabled(rename = "Task")]
    task: String,
    #[tabled(rename = "Age")]
    age: String,
}

/// Parse `git worktree list --porcelain` output into (path, branch) pairs.
fn parse_worktrees(output: &str) -> Vec<(PathBuf, String)> {
    let mut result = Vec::new();
    for block in output.split("\n\n") {
        let mut path: Option<PathBuf> = None;
        let mut branch: Option<String> = None;
        for line in block.lines() {
            if let Some(p) = line.strip_prefix("worktree ") {
                path = Some(PathBuf::from(p.trim()));
            } else if let Some(b) = line.strip_prefix("branch ") {
                branch = Some(
                    b.trim()
                        .strip_prefix("refs/heads/")
                        .unwrap_or(b.trim())
                        .to_string(),
                );
            }
        }
        if let (Some(p), Some(b)) = (path, branch) {
            result.push((p, b));
        }
    }
    result
}

/// Read all session files from `~/.hex/sessions/agent-*.json`.
/// Returns a map of `worktree_branch` → session JSON value.
fn load_sessions() -> HashMap<String, Value> {
    let mut map = HashMap::new();

    let sessions_dir = match dirs::home_dir() {
        Some(h) => h.join(".hex").join("sessions"),
        None => return map,
    };

    let read_dir = match std::fs::read_dir(&sessions_dir) {
        Ok(d) => d,
        Err(_) => return map,
    };

    for entry in read_dir.flatten() {
        let fname = entry.file_name();
        let name = fname.to_string_lossy();
        if !name.starts_with("agent-") || !name.ends_with(".json") {
            continue;
        }
        if let Ok(contents) = std::fs::read_to_string(entry.path()) {
            if let Ok(json) = serde_json::from_str::<Value>(&contents) {
                if let Some(branch) = json["worktree_branch"].as_str() {
                    map.insert(branch.to_string(), json);
                }
            }
        }
    }
    map
}

/// Format elapsed duration as "Xm", "Xh", "Xd", or "-".
fn format_age(elapsed: Duration) -> String {
    let secs = elapsed.as_secs();
    if secs < 60 {
        format!("{}s", secs)
    } else if secs < 3600 {
        format!("{}m", secs / 60)
    } else if secs < 86400 {
        format!("{}h", secs / 3600)
    } else {
        format!("{}d", secs / 86400)
    }
}

/// Get the mtime of a path as elapsed duration from now.
fn path_age(path: &PathBuf) -> Option<Duration> {
    let meta = std::fs::metadata(path).ok()?;
    let modified = meta.modified().ok()?;
    SystemTime::now().duration_since(modified).ok()
}

pub async fn run() -> anyhow::Result<()> {
    use std::process::Command;

    println!("{} Worktree Audit", "\u{2b21}".cyan());
    println!();

    // 1. Run git worktree list --porcelain
    let output = Command::new("git")
        .args(["worktree", "list", "--porcelain"])
        .output()?;

    if !output.status.success() {
        anyhow::bail!("git worktree list failed: {}", String::from_utf8_lossy(&output.stderr));
    }

    let porcelain = String::from_utf8_lossy(&output.stdout);
    let worktrees = parse_worktrees(&porcelain);

    if worktrees.is_empty() {
        println!("  {} No worktrees found.", "\u{25cb}".dimmed());
        return Ok(());
    }

    // 2. Load session files
    let sessions = load_sessions();

    // 3. Build table rows
    let mut rows: Vec<WorktreeRow> = Vec::new();

    for (wt_path, branch) in &worktrees {
        let dir_name = wt_path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| wt_path.to_string_lossy().to_string());

        let is_main = branch == "main" || branch == "master";

        let session = sessions.get(branch.as_str());

        let agent_col = if is_main {
            "-".dimmed().to_string()
        } else {
            match session.and_then(|s| s["agent_id"].as_str()) {
                Some(id) => {
                    // shorten to first 12 chars for readability
                    let short = if id.len() > 12 { &id[..12] } else { id };
                    short.green().to_string()
                }
                None => "(unassigned)".dimmed().to_string(),
            }
        };

        let task_col = if is_main {
            "-".dimmed().to_string()
        } else {
            match session.and_then(|s| s["current_task_id"].as_str()) {
                Some(t) => {
                    let short = if t.len() > 14 { &t[..14] } else { t };
                    short.yellow().to_string()
                }
                None => "-".dimmed().to_string(),
            }
        };

        // Age: prefer session file mtime, fall back to worktree dir mtime
        let age_col = if is_main {
            "-".dimmed().to_string()
        } else {
            // Try to find the session file path for this branch
            let age = session
                .and_then(|s| {
                    // session file named after agent_id
                    let agent_id = s["agent_id"].as_str()?;
                    let session_path = dirs::home_dir()?
                        .join(".hex")
                        .join("sessions")
                        .join(format!("agent-{}.json", agent_id));
                    path_age(&session_path)
                })
                .or_else(|| path_age(wt_path));

            match age {
                Some(d) => format_age(d),
                None => "-".to_string(),
            }
        };

        let worktree_col = if is_main {
            format!("  ({})", dir_name).dimmed().to_string()
        } else {
            dir_name
        };

        let branch_col = if is_main {
            branch.dimmed().to_string()
        } else {
            branch.clone()
        };

        rows.push(WorktreeRow {
            worktree: worktree_col,
            branch: branch_col,
            agent: agent_col,
            task: task_col,
            age: age_col,
        });
    }

    println!("{}", HexTable::compact(&rows));
    println!();
    println!(
        "  {} worktree(s) — {} with assigned agent",
        rows.len(),
        rows.iter()
            .filter(|r| !r.agent.contains('-') || r.agent.contains('\x1b'))
            .count()
    );

    Ok(())
}

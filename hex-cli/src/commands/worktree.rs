//! Git worktree management commands.
//!
//! `hex worktree list|merge|cleanup` — manage feature worktrees.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::process::Command;

use clap::Subcommand;
use colored::Colorize;

#[derive(Subcommand)]
pub enum WorktreeAction {
    /// List all active worktrees
    List,
    /// Merge worktree branches into main (file-level cherry-pick)
    Merge {
        /// Branch pattern to match (e.g. "feat/auth" matches all feat/auth/* branches)
        #[arg(value_name = "PATTERN")]
        pattern: Option<String>,
        /// Merge all non-main worktree branches
        #[arg(long)]
        all: bool,
        /// Actually perform the merge (default is dry-run)
        #[arg(long)]
        force: bool,
    },
    /// Remove worktrees whose branches are already merged into main
    Cleanup {
        /// Actually remove (default is dry-run)
        #[arg(long)]
        force: bool,
    },
}

pub async fn run(action: WorktreeAction) -> anyhow::Result<()> {
    match action {
        WorktreeAction::List => list().await,
        WorktreeAction::Merge { pattern, all, force } => merge(pattern, all, force).await,
        WorktreeAction::Cleanup { force } => cleanup(force).await,
    }
}

// ── Helpers ────────────────────────────────────────────

fn repo_root() -> anyhow::Result<String> {
    let output = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .output()?;
    if !output.status.success() {
        anyhow::bail!("Not inside a git repository");
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn main_branch() -> String {
    let output = Command::new("git")
        .args(["branch", "--list", "main"])
        .output()
        .ok();
    if let Some(o) = output {
        if !String::from_utf8_lossy(&o.stdout).trim().is_empty() {
            return "main".to_string();
        }
    }
    "master".to_string()
}

/// Parse `git worktree list --porcelain` into (path, branch) pairs.
fn parse_worktrees() -> anyhow::Result<Vec<(String, String)>> {
    let output = Command::new("git")
        .args(["worktree", "list", "--porcelain"])
        .output()?;
    if !output.status.success() {
        anyhow::bail!(
            "git worktree list failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut results = Vec::new();
    let mut current_path = String::new();
    let mut current_branch = String::new();

    for line in stdout.lines() {
        if line.starts_with("worktree ") {
            if !current_path.is_empty() && !current_branch.is_empty() {
                results.push((current_path.clone(), current_branch.clone()));
            }
            current_path = line.strip_prefix("worktree ").unwrap_or("").to_string();
            current_branch.clear();
        } else if line.starts_with("branch ") {
            let raw = line.strip_prefix("branch ").unwrap_or("");
            current_branch = raw
                .strip_prefix("refs/heads/")
                .unwrap_or(raw)
                .to_string();
        } else if line == "detached" {
            current_branch = "(detached)".to_string();
        } else if line.is_empty() {
            if !current_path.is_empty() && !current_branch.is_empty() {
                results.push((current_path.clone(), current_branch.clone()));
            }
            current_path.clear();
            current_branch.clear();
        }
    }
    // Last entry
    if !current_path.is_empty() && !current_branch.is_empty() {
        results.push((current_path, current_branch));
    }

    Ok(results)
}

/// Get changed files for a branch vs main.
fn changed_files(branch: &str, main: &str) -> anyhow::Result<Vec<String>> {
    let output = Command::new("git")
        .args(["diff", &format!("{}...{}", main, branch), "--name-only"])
        .output()?;
    if !output.status.success() {
        anyhow::bail!(
            "git diff failed for branch {}: {}",
            branch,
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter(|l| !l.is_empty())
        .map(|l| l.to_string())
        .collect())
}

/// Get the Unix timestamp of the latest commit on a branch.
fn latest_commit_epoch(branch: &str) -> anyhow::Result<i64> {
    let output = Command::new("git")
        .args(["log", "-1", "--format=%ct", branch])
        .output()?;
    if !output.status.success() {
        anyhow::bail!(
            "git log failed for branch {}: {}",
            branch,
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let ts = String::from_utf8_lossy(&output.stdout).trim().to_string();
    ts.parse::<i64>()
        .map_err(|e| anyhow::anyhow!("Failed to parse timestamp for {}: {}", branch, e))
}

/// Check if a branch is merged into main.
fn is_merged(branch: &str, main: &str) -> bool {
    let output = Command::new("git")
        .args(["branch", "--merged", main])
        .output()
        .ok();
    if let Some(o) = output {
        let stdout = String::from_utf8_lossy(&o.stdout);
        return stdout
            .lines()
            .any(|l| l.trim_start_matches(['*', '+', ' ']).trim() == branch);
    }
    false
}

// ── Subcommands ────────────────────────────────────────

async fn list() -> anyhow::Result<()> {
    let worktrees = parse_worktrees()?;
    let main = main_branch();

    println!("{}", "Worktrees".bold());
    println!("{}", "\u{2500}".repeat(60));

    if worktrees.is_empty() {
        println!("  No worktrees found.");
        return Ok(());
    }

    for (path, branch) in &worktrees {
        let is_main = branch == &main;
        let merged = if !is_main { is_merged(branch, &main) } else { false };
        let indicator = if is_main {
            "\u{25cf}".green().to_string()
        } else if merged {
            "\u{2713}".green().to_string()
        } else {
            "\u{25cb}".yellow().to_string()
        };
        let merged_tag = if merged && !is_main {
            " (merged)".green().to_string()
        } else {
            String::new()
        };
        let display_branch = if is_main {
            branch.green().bold().to_string()
        } else {
            branch.cyan().to_string()
        };
        println!("  {} {} {}{}", indicator, display_branch, path.dimmed(), merged_tag);
    }

    Ok(())
}

async fn merge(
    pattern: Option<String>,
    all: bool,
    force: bool,
) -> anyhow::Result<()> {
    if pattern.is_none() && !all {
        anyhow::bail!("Specify a branch pattern or use --all");
    }

    let worktrees = parse_worktrees()?;
    let main = main_branch();

    // Filter worktrees by pattern
    let candidates: Vec<(String, String)> = worktrees
        .into_iter()
        .filter(|(_, branch)| {
            if branch == &main || branch == "(detached)" {
                return false;
            }
            if all {
                return true;
            }
            if let Some(ref pat) = pattern {
                branch.contains(pat)
            } else {
                false
            }
        })
        .collect();

    if candidates.is_empty() {
        println!("  No matching worktree branches found.");
        return Ok(());
    }

    println!("{}", "Worktree Merge".bold());
    println!("{}", "\u{2500}".repeat(60));

    // Collect changed files per branch
    let mut branch_files: HashMap<String, Vec<String>> = HashMap::new();
    for (_, branch) in &candidates {
        match changed_files(branch, &main) {
            Ok(files) => {
                if !files.is_empty() {
                    branch_files.insert(branch.clone(), files);
                } else {
                    println!("  {} {} — no changes vs main", "\u{2013}".dimmed(), branch.dimmed());
                }
            }
            Err(e) => {
                println!("  {} {} — {}", "\u{2717}".red(), branch, e);
            }
        }
    }

    if branch_files.is_empty() {
        println!("\n  No files to merge.");
        return Ok(());
    }

    // Detect file overlaps between branches
    let mut file_owners: HashMap<String, Vec<String>> = HashMap::new();
    for (branch, files) in &branch_files {
        for file in files {
            file_owners
                .entry(file.clone())
                .or_default()
                .push(branch.clone());
        }
    }

    let overlapping: HashMap<&String, &Vec<String>> = file_owners
        .iter()
        .filter(|(_, owners)| owners.len() > 1)
        .collect();

    // For overlapping files, pick the branch with the most recent commit
    let mut overlap_winner: HashMap<String, String> = HashMap::new();
    if !overlapping.is_empty() {
        println!("\n  {} File overlaps detected — resolving by most recent commit:", "\u{26a0}".yellow());

        // Cache branch timestamps
        let mut branch_ts: HashMap<String, i64> = HashMap::new();
        for branch in branch_files.keys() {
            if let Ok(ts) = latest_commit_epoch(branch) {
                branch_ts.insert(branch.clone(), ts);
            }
        }

        for (file, owners) in &overlapping {
            // Find the owner with the highest (most recent) timestamp
            let winner = owners
                .iter()
                .max_by_key(|b| branch_ts.get(*b).copied().unwrap_or(0))
                .cloned()
                .unwrap_or_else(|| owners[0].clone());
            println!(
                "    {} owned by [{}] → picking {}",
                file.yellow(),
                owners.join(", ").dimmed(),
                winner.cyan()
            );
            overlap_winner.insert((*file).clone(), winner);
        }
    }

    // Build the set of files that are overlapping (handled separately)
    let overlap_files: HashSet<&String> = overlapping.keys().copied().collect();

    // Determine merge order by tier prefix (P0 < P1 < P2 ...)
    let mut ordered_branches: Vec<(u32, String)> = branch_files
        .keys()
        .map(|b| {
            // Try to extract tier from branch name (e.g. feat/foo/p0-domain -> 0)
            let tier = b
                .split('/')
                .last()
                .and_then(|seg| {
                    let seg_lower = seg.to_lowercase();
                    if seg_lower.starts_with('p') {
                        seg_lower[1..]
                            .chars()
                            .take_while(|c| c.is_ascii_digit())
                            .collect::<String>()
                            .parse::<u32>()
                            .ok()
                    } else {
                        None
                    }
                })
                .unwrap_or(99);
            (tier, b.clone())
        })
        .collect();
    ordered_branches.sort_by_key(|(tier, _)| *tier);

    // Show plan
    println!("\n  Merge order (non-overlapping, by tier):");
    let mut total_files = 0usize;
    for (tier, branch) in &ordered_branches {
        let files = &branch_files[branch];
        let safe_files: Vec<&String> = files.iter().filter(|f| !overlap_files.contains(f)).collect();
        if safe_files.is_empty() {
            continue;
        }
        println!(
            "    {} {} — {} file(s)",
            format!("T{}", tier).cyan(),
            branch,
            safe_files.len()
        );
        for f in &safe_files {
            println!("      {}", f);
        }
        total_files += safe_files.len();
    }

    if !overlap_winner.is_empty() {
        println!("\n  Overlap resolution (most recent branch wins):");
        for (file, winner) in &overlap_winner {
            println!("    {} ← {}", file, winner.cyan());
        }
        total_files += overlap_winner.len();
    }

    if !force {
        println!(
            "\n  {} Dry run — {} file(s) would be merged. Pass {} to execute.",
            "\u{26a0}".yellow(),
            total_files,
            "--force".bold()
        );
        return Ok(());
    }

    // Execute merge — Phase 1: non-overlapping files in tier order
    let root = repo_root()?;
    let root_path = Path::new(&root);
    let mut merged_count = 0usize;
    let mut errors = Vec::new();

    for (_tier, branch) in &ordered_branches {
        let files = &branch_files[branch];
        let safe_files: Vec<String> = files
            .iter()
            .filter(|f| !overlap_files.contains(f))
            .cloned()
            .collect();

        if safe_files.is_empty() {
            continue;
        }

        // git checkout <branch> -- <files>
        let mut args = vec!["checkout".to_string(), branch.clone(), "--".to_string()];
        args.extend(safe_files.iter().cloned());

        let output = Command::new("git")
            .args(&args)
            .current_dir(root_path)
            .output()?;

        if output.status.success() {
            println!(
                "  {} Merged {} file(s) from {}",
                "\u{2713}".green(),
                safe_files.len(),
                branch.cyan()
            );
            merged_count += safe_files.len();
        } else {
            let err = String::from_utf8_lossy(&output.stderr).trim().to_string();
            println!("  {} Failed to merge from {}: {}", "\u{2717}".red(), branch, err);
            errors.push((branch.clone(), err));
        }
    }

    // Execute merge — Phase 2: overlapping files from winner branches
    // Group overlap files by winner branch for batch checkout
    let mut winner_files: HashMap<String, Vec<String>> = HashMap::new();
    for (file, winner) in &overlap_winner {
        winner_files
            .entry(winner.clone())
            .or_default()
            .push(file.clone());
    }

    for (branch, files) in &winner_files {
        let mut args = vec!["checkout".to_string(), branch.clone(), "--".to_string()];
        args.extend(files.iter().cloned());

        let output = Command::new("git")
            .args(&args)
            .current_dir(root_path)
            .output()?;

        if output.status.success() {
            println!(
                "  {} Cherry-picked {} overlapping file(s) from {}",
                "\u{2713}".green(),
                files.len(),
                branch.cyan()
            );
            merged_count += files.len();
        } else {
            let err = String::from_utf8_lossy(&output.stderr).trim().to_string();
            println!(
                "  {} Failed to cherry-pick from {}: {}",
                "\u{2717}".red(),
                branch,
                err
            );
            errors.push((branch.clone(), err));
        }
    }

    // Run cargo check as gate
    println!("\n  Running cargo check --workspace ...");
    let check = Command::new("cargo")
        .args(["check", "--workspace"])
        .current_dir(root_path)
        .status()?;

    if check.success() {
        println!("  {} cargo check passed", "\u{2713}".green());
    } else {
        println!(
            "  {} cargo check FAILED — merged files may have conflicts",
            "\u{2717}".red()
        );
    }

    // Summary
    println!("\n  {} Merged {} file(s) from {} branch(es).",
        "\u{2500}".repeat(3),
        merged_count,
        ordered_branches.len()
    );
    if !errors.is_empty() {
        println!("  {} error(s) encountered.", errors.len());
    }

    Ok(())
}

async fn cleanup(force: bool) -> anyhow::Result<()> {
    let worktrees = parse_worktrees()?;
    let main = main_branch();

    println!("{}", "Worktree Cleanup".bold());
    println!("{}", "\u{2500}".repeat(60));

    let merged_wts: Vec<(String, String)> = worktrees
        .into_iter()
        .filter(|(_, branch)| {
            branch != &main && branch != "(detached)" && is_merged(branch, &main)
        })
        .collect();

    if merged_wts.is_empty() {
        println!("  {} No merged worktrees to clean up.", "\u{2713}".green());
        return Ok(());
    }

    println!("  Found {} merged worktree(s):", merged_wts.len());
    for (path, branch) in &merged_wts {
        println!("    {} {} ({})", "\u{2013}".green(), branch.cyan(), path.dimmed());
    }

    if !force {
        println!(
            "\n  {} Dry run — pass {} to remove.",
            "\u{26a0}".yellow(),
            "--force".bold()
        );
        return Ok(());
    }

    let mut removed = 0usize;
    for (path, branch) in &merged_wts {
        // Remove worktree
        let wt_result = Command::new("git")
            .args(["worktree", "remove", path])
            .output()?;

        if wt_result.status.success() {
            // Delete branch
            let _ = Command::new("git")
                .args(["branch", "-d", branch])
                .output();
            println!("  {} Removed {} (branch {})", "\u{2713}".green(), path, branch);
            removed += 1;
        } else {
            let err = String::from_utf8_lossy(&wt_result.stderr);
            println!("  {} Failed to remove {}: {}", "\u{2717}".red(), path, err.trim());
        }
    }

    println!("\n  Removed {} worktree(s).", removed);
    Ok(())
}

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
    List {
        /// Only list worktrees with no commits in the last 24h (drift signal
        /// for the improver `git_drift` detector).
        #[arg(long)]
        stale: bool,
        /// Emit findings as JSON for the improver detector pipeline
        /// (`{findings: [{branch, path, age_hours}]}`).
        #[arg(long)]
        json: bool,
    },
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
    /// Show pending merge requests + vote tallies (ADR-2605081126)
    Status {
        /// Emit JSON instead of the colorized table
        #[arg(long)]
        json: bool,
    },
    /// Operator override: approve a merge request and let the integrator merge
    Approve {
        /// Worktree path of the merge request to approve
        #[arg(value_name = "PATH")]
        path: String,
        /// Reason for the override (logged in merge_vote.reason)
        #[arg(long, default_value = "operator approval")]
        reason: String,
    },
    /// Reject a merge request — voter=operator verdict=fail
    Reject {
        /// Worktree path of the merge request to reject
        #[arg(value_name = "PATH")]
        path: String,
        /// Reason explaining the rejection (free text)
        #[arg(value_name = "REASON")]
        reason: String,
    },
}

pub async fn run(action: WorktreeAction) -> anyhow::Result<()> {
    match action {
        WorktreeAction::List { stale, json } => list(stale, json).await,
        WorktreeAction::Merge { pattern, all, force } => merge(pattern, all, force).await,
        WorktreeAction::Cleanup { force } => cleanup(force).await,
        WorktreeAction::Status { json } => merge_team_status(json).await,
        WorktreeAction::Approve { path, reason } => merge_team_approve(path, reason).await,
        WorktreeAction::Reject { path, reason } => merge_team_reject(path, reason).await,
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

async fn list(stale: bool, json: bool) -> anyhow::Result<()> {
    let worktrees = parse_worktrees()?;
    let main = main_branch();

    // Compute hours since the most recent commit on each worktree's branch.
    // A worktree with >24h since last commit is "stale" — likely abandoned
    // mid-feature or already merged elsewhere. Branches we can't query (e.g.
    // detached) get u64::MAX so the --stale filter treats them as stale too.
    let last_commit_hours = |branch: &str| -> u64 {
        let out = Command::new("git")
            .args(["log", "-1", "--format=%ct", branch])
            .output();
        let Ok(out) = out else { return u64::MAX };
        let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
        let Ok(committed_at) = s.parse::<i64>() else { return u64::MAX };
        let now = chrono::Utc::now().timestamp();
        ((now - committed_at).max(0) / 3600) as u64
    };

    let mut entries: Vec<(String, String, u64, bool)> = worktrees
        .iter()
        .map(|(path, branch)| {
            let age = if branch == &main { 0 } else { last_commit_hours(branch) };
            let is_stale = branch != &main && age > 24;
            (path.clone(), branch.clone(), age, is_stale)
        })
        .collect();
    if stale {
        entries.retain(|e| e.3);
    }

    if json {
        let findings: Vec<_> = entries
            .iter()
            .map(|(path, branch, age, is_stale)| {
                serde_json::json!({
                    "branch": branch,
                    "path": path,
                    "age_hours": age,
                    "stale": is_stale,
                    "severity": if *is_stale { "warning" } else { "info" },
                })
            })
            .collect();
        println!("{}", serde_json::json!({"findings": findings}));
        return Ok(());
    }

    println!("{}", "Worktrees".bold());
    println!("{}", "\u{2500}".repeat(60));

    if entries.is_empty() {
        println!("  No worktrees found.");
        return Ok(());
    }

    for (path, branch, _age, _is_stale) in &entries {
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

    // ── Merge Integrity Check (ADR-2604131800) ─────────────────────────
    // Verify every worktree's added lines are present on main after merge.
    // This catches the "last write wins" problem where git checkout from
    // one branch silently drops additions from another branch.
    println!("\n  Verifying merge integrity...");
    let mut integrity_failures: Vec<(String, String, usize)> = Vec::new(); // (branch, file, missing_lines)

    for (_, branch) in &ordered_branches {
        // Get what this branch added vs main (before our merge)
        let diff_output = Command::new("git")
            .args(["diff", &format!("{}...{}", main, branch), "--unified=0"])
            .current_dir(root_path)
            .output();

        let diff_text = match diff_output {
            Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).to_string(),
            _ => continue,
        };

        // Parse added lines from the diff (lines starting with + but not +++)
        let mut current_file = String::new();
        let mut missing_count = 0usize;

        for line in diff_text.lines() {
            if line.starts_with("+++ b/") {
                // Check previous file's missing lines
                if missing_count > 0 {
                    integrity_failures.push((branch.clone(), current_file.clone(), missing_count));
                }
                current_file = line.strip_prefix("+++ b/").unwrap_or("").to_string();
                missing_count = 0;
            } else if line.starts_with('+') && !line.starts_with("+++") {
                // This is an added line — verify it exists in the working tree
                let added_content = &line[1..]; // strip the leading +
                if added_content.trim().is_empty() {
                    continue; // skip blank lines
                }
                // Read the file from working tree and check if the line is present
                let file_path = root_path.join(&current_file);
                if let Ok(contents) = std::fs::read_to_string(&file_path) {
                    if !contents.contains(added_content.trim()) {
                        missing_count += 1;
                    }
                }
            }
        }
        // Check last file
        if missing_count > 0 {
            integrity_failures.push((branch.clone(), current_file, missing_count));
        }
    }

    if !integrity_failures.is_empty() {
        println!(
            "\n  {} MERGE INCOMPLETE — code from worktree agents was dropped:",
            "\u{2717}".red().bold()
        );
        for (branch, file, count) in &integrity_failures {
            println!(
                "    {} {} lines from {} missing in {}",
                "\u{2717}".red(),
                count,
                branch.cyan(),
                file.yellow()
            );
        }
        println!(
            "\n  Fix: use {} on the affected files to manually merge both branches' changes.",
            "git merge <branch>".bold()
        );
        // Don't proceed to cargo check — merge is incomplete
        anyhow::bail!(
            "Merge integrity check failed: {} file(s) have missing lines from worktree agents",
            integrity_failures.len()
        );
    } else {
        println!(
            "  {} All worktree additions verified on main",
            "\u{2713}".green()
        );
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
    let stale_threshold = std::time::Duration::from_secs(24 * 60 * 60);

    println!("{}", "Worktree Cleanup".bold());
    println!("{}", "\u{2500}".repeat(60));

    // Find merged worktrees
    let merged_wts: Vec<(String, String)> = worktrees
        .iter()
        .filter(|(_, branch)| {
            branch != &main && branch != "(detached)" && is_merged(branch, &main)
        })
        .cloned()
        .collect();

    // Find stale worktrees (24h+ no commits)
    let stale_wts: Vec<(String, String)> = worktrees
        .iter()
        .filter(|(path, branch)| {
            if branch == &main || branch == "(detached)" {
                return false;
            }
            // Check if branch is a feature-type branch
            let is_feature = branch.starts_with("feat/")
                || branch.starts_with("hex/")
                || branch.starts_with("worktree-")
                || branch.starts_with("claude/");
            if !is_feature {
                return false;
            }
            // Check last commit age
            if let Ok(output) = Command::new("git")
                .args(["log", "-1", "--format=%ct", branch])
                .output()
            {
                if output.status.success() {
                    if let Ok(ts) = String::from_utf8_lossy(&output.stdout).trim().parse::<u64>() {
                        let now = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .map(|d| d.as_secs())
                            .unwrap_or(0);
                        return now.saturating_sub(ts) > stale_threshold.as_secs();
                    }
                }
            }
            false
        })
        .cloned()
        .collect();

    let all_to_clean: Vec<(String, String)> = merged_wts
        .into_iter()
        .chain(stale_wts.into_iter())
        .collect();

    if all_to_clean.is_empty() {
        println!("  {} No merged or stale worktrees to clean up.", "\u{2713}".green());
        return Ok(());
    }

    let merged_count = all_to_clean.len();
    println!("  Found {} worktree(s) to clean up:", merged_count);
    for (path, branch) in &all_to_clean {
        let status = if is_merged(branch, &main) { "merged" } else { "stale" };
        println!("    {} {} ({}) [{}]", "\u{2013}".green(), branch.cyan(), path.dimmed(), status);
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
    for (path, branch) in &all_to_clean {
        // Remove worktree (use --force for stale worktrees with untracked files)
        let wt_result = Command::new("git")
            .args(["worktree", "remove", "--force", path])
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

// ============================================================
//  Merge-team CLI surface (ADR-2605081126 P5)
// ============================================================

const STDB_HOST_DEFAULT: &str = "http://127.0.0.1:3033";

fn stdb_host() -> String {
    std::env::var("HEX_SPACETIMEDB_HOST").unwrap_or_else(|_| STDB_HOST_DEFAULT.to_string())
}

fn hex_db() -> String {
    std::env::var("HEX_STDB_DATABASE")
        .unwrap_or_else(|_| hex_core::stdb_database_for_module("hexflo-coordination").to_string())
}

#[derive(Debug, Clone)]
struct MergeRequestRow {
    worktree_path: String,
    branch: String,
    role: String,
    opened_at: String,
    status: String,
    related_workplan: String,
}

#[derive(Debug, Clone)]
struct MergeVoteRow {
    voter: String,
    verdict: String,
    reason: String,
}

async fn merge_team_status(json: bool) -> anyhow::Result<()> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()?;
    let requests = fetch_merge_requests(&client).await?;
    let votes = fetch_merge_votes(&client).await?;

    if json {
        let payload = serde_json::json!({
            "requests": requests
                .iter()
                .map(|r| {
                    let v: Vec<serde_json::Value> = votes
                        .iter()
                        .filter(|(p, _)| p == &r.worktree_path)
                        .map(|(_, v)| serde_json::json!({
                            "voter": v.voter,
                            "verdict": v.verdict,
                            "reason": v.reason,
                        }))
                        .collect();
                    serde_json::json!({
                        "worktree_path": r.worktree_path,
                        "branch": r.branch,
                        "role": r.role,
                        "status": r.status,
                        "related_workplan": r.related_workplan,
                        "votes": v,
                    })
                })
                .collect::<Vec<_>>()
        });
        println!("{}", serde_json::to_string_pretty(&payload)?);
        return Ok(());
    }

    if requests.is_empty() {
        println!("  No active merge requests.");
        return Ok(());
    }

    println!(
        "{}",
        format!("⬡ Merge requests ({})", requests.len()).bold()
    );
    println!();
    for r in &requests {
        let status_color = match r.status.as_str() {
            "pending" => r.status.yellow(),
            "voting" => r.status.cyan(),
            "approved" => r.status.green(),
            "rejected" => r.status.red(),
            "merged" => r.status.dimmed(),
            _ => r.status.normal(),
        };
        println!(
            "  {} {}  {}",
            r.worktree_path.bold(),
            format!("[{}]", r.branch).dimmed(),
            status_color
        );
        println!(
            "    role={}  workplan={}  opened={}",
            r.role, r.related_workplan.dimmed(), r.opened_at[..r.opened_at.len().min(60)].dimmed()
        );
        let role_votes: Vec<&(String, MergeVoteRow)> = votes
            .iter()
            .filter(|(p, _)| p == &r.worktree_path)
            .collect();
        if role_votes.is_empty() {
            println!("    {}", "(no votes yet)".dimmed());
        } else {
            for (_, v) in role_votes {
                let v_color = match v.verdict.as_str() {
                    "pass" => v.verdict.green(),
                    "fail" => v.verdict.red(),
                    "abstain" => v.verdict.yellow(),
                    _ => v.verdict.normal(),
                };
                let reason_short = v.reason.chars().take(80).collect::<String>();
                println!(
                    "    {:18} {} {}",
                    v.voter,
                    v_color,
                    if reason_short.is_empty() {
                        String::new()
                    } else {
                        format!("— {}", reason_short.dimmed())
                    }
                );
            }
        }
        println!();
    }

    println!(
        "  Tip: `hex worktree approve <path>` overrides; `hex worktree reject <path> <reason>` rejects."
    );
    Ok(())
}

async fn merge_team_approve(path: String, reason: String) -> anyhow::Result<()> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()?;
    cast_operator_vote(&client, &path, "pass", &reason).await?;
    println!(
        "  {} Operator override recorded for {} ({})",
        "\u{2713}".green(),
        path.bold(),
        reason.dimmed()
    );
    println!(
        "  Integrator subscriber will pick up status=approved on next tick (~5s) and run hex worktree merge."
    );
    Ok(())
}

async fn merge_team_reject(path: String, reason: String) -> anyhow::Result<()> {
    if reason.trim().is_empty() {
        anyhow::bail!("reject requires a non-empty reason");
    }
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()?;
    cast_operator_vote(&client, &path, "fail", &reason).await?;
    println!(
        "  {} Rejected {} — {}",
        "\u{2717}".red(),
        path.bold(),
        reason
    );
    Ok(())
}

async fn cast_operator_vote(
    client: &reqwest::Client,
    path: &str,
    verdict: &str,
    reason: &str,
) -> anyhow::Result<()> {
    let url = format!("{}/v1/database/{}/call/merge_vote_cast", stdb_host(), hex_db());
    let body = serde_json::json!([path, "operator", verdict, reason]);
    let resp = client.post(&url).json(&body).send().await?;
    if !resp.status().is_success() {
        let body_text = resp.text().await.unwrap_or_default();
        anyhow::bail!("merge_vote_cast HTTP {}: {}", body_text, body_text);
    }
    // Trigger the tally so status flips immediately.
    let tally_url = format!(
        "{}/v1/database/{}/call/merge_decision_tally",
        stdb_host(),
        hex_db()
    );
    let _ = client.post(&tally_url).json(&[path]).send().await;
    Ok(())
}

async fn fetch_merge_requests(client: &reqwest::Client) -> anyhow::Result<Vec<MergeRequestRow>> {
    let q = "SELECT worktree_path, branch, role, opened_at, status, related_workplan FROM merge_request";
    let url = format!("{}/v1/database/{}/sql", stdb_host(), hex_db());
    let resp = client
        .post(&url)
        .header("Content-Type", "text/plain")
        .body(q)
        .send()
        .await?;
    if !resp.status().is_success() {
        anyhow::bail!("merge_request query HTTP {}", resp.status());
    }
    let body: serde_json::Value = resp.json().await?;
    let rows = body
        .as_array()
        .and_then(|a| a.first())
        .and_then(|t| t.get("rows"))
        .and_then(|r| r.as_array())
        .cloned()
        .unwrap_or_default();
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let cols = match row.as_array() {
            Some(a) if a.len() >= 6 => a,
            _ => continue,
        };
        out.push(MergeRequestRow {
            worktree_path: cols[0].as_str().unwrap_or("").to_string(),
            branch: cols[1].as_str().unwrap_or("").to_string(),
            role: cols[2].as_str().unwrap_or("").to_string(),
            opened_at: cols[3].as_str().unwrap_or("").to_string(),
            status: cols[4].as_str().unwrap_or("").to_string(),
            related_workplan: cols[5].as_str().unwrap_or("").to_string(),
        });
    }
    out.sort_by(|a, b| a.worktree_path.cmp(&b.worktree_path));
    Ok(out)
}

async fn fetch_merge_votes(
    client: &reqwest::Client,
) -> anyhow::Result<Vec<(String, MergeVoteRow)>> {
    let q = "SELECT worktree_path, voter, verdict, reason FROM merge_vote";
    let url = format!("{}/v1/database/{}/sql", stdb_host(), hex_db());
    let resp = client
        .post(&url)
        .header("Content-Type", "text/plain")
        .body(q)
        .send()
        .await?;
    if !resp.status().is_success() {
        return Ok(Vec::new()); // table may be empty / not yet populated
    }
    let body: serde_json::Value = resp.json().await?;
    let rows = body
        .as_array()
        .and_then(|a| a.first())
        .and_then(|t| t.get("rows"))
        .and_then(|r| r.as_array())
        .cloned()
        .unwrap_or_default();
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let cols = match row.as_array() {
            Some(a) if a.len() >= 4 => a,
            _ => continue,
        };
        let path = cols[0].as_str().unwrap_or("").to_string();
        out.push((
            path,
            MergeVoteRow {
                voter: cols[1].as_str().unwrap_or("").to_string(),
                verdict: cols[2].as_str().unwrap_or("").to_string(),
                reason: cols[3].as_str().unwrap_or("").to_string(),
            },
        ));
    }
    Ok(out)
}

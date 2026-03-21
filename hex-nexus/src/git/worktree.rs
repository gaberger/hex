//! Git worktree listing and management.
//!
//! Uses git CLI for worktree operations since libgit2's worktree support is limited.

use std::path::Path;

use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorktreeInfo {
    pub path: String,
    pub branch: String,
    pub head_sha: String,
    pub is_main: bool,
    pub is_bare: bool,
    pub commit_count: Option<usize>,
}

/// List all worktrees for a repository using `git worktree list --porcelain`.
pub fn list_worktrees(root_path: &Path) -> Result<Vec<WorktreeInfo>, String> {
    let output = std::process::Command::new("git")
        .args(["worktree", "list", "--porcelain"])
        .current_dir(root_path)
        .output()
        .map_err(|e| format!("Failed to run git worktree list: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("git worktree list failed: {}", stderr));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    parse_porcelain_worktrees(&stdout, root_path)
}

fn parse_porcelain_worktrees(output: &str, main_path: &Path) -> Result<Vec<WorktreeInfo>, String> {
    let mut worktrees = Vec::new();
    let mut current_path = String::new();
    let mut current_sha = String::new();
    let mut current_branch = String::new();
    let mut is_bare = false;

    for line in output.lines() {
        if line.starts_with("worktree ") {
            // If we have a pending worktree, push it
            if !current_path.is_empty() {
                let is_main = Path::new(&current_path) == main_path
                    || Path::new(&current_path) == main_path.join(".git");
                worktrees.push(WorktreeInfo {
                    path: current_path.clone(),
                    branch: current_branch.clone(),
                    head_sha: current_sha.clone(),
                    is_main,
                    is_bare,
                    commit_count: None,
                });
            }
            current_path = line.strip_prefix("worktree ").unwrap_or("").to_string();
            current_sha.clear();
            current_branch.clear();
            is_bare = false;
        } else if line.starts_with("HEAD ") {
            current_sha = line.strip_prefix("HEAD ").unwrap_or("").to_string();
        } else if line.starts_with("branch ") {
            let raw = line.strip_prefix("branch ").unwrap_or("");
            // Strip refs/heads/ prefix
            current_branch = raw
                .strip_prefix("refs/heads/")
                .unwrap_or(raw)
                .to_string();
        } else if line == "bare" {
            is_bare = true;
        } else if line == "detached" {
            current_branch = "(detached)".to_string();
        } else if line.is_empty() {
            // End of worktree block — push if we have data
            if !current_path.is_empty() {
                let is_main = Path::new(&current_path) == main_path;
                worktrees.push(WorktreeInfo {
                    path: current_path.clone(),
                    branch: current_branch.clone(),
                    head_sha: current_sha.clone(),
                    is_main,
                    is_bare,
                    commit_count: None,
                });
                current_path.clear();
                current_sha.clear();
                current_branch.clear();
                is_bare = false;
            }
        }
    }

    // Push last entry if not yet pushed
    if !current_path.is_empty() {
        let is_main = Path::new(&current_path) == main_path;
        worktrees.push(WorktreeInfo {
            path: current_path,
            branch: current_branch,
            head_sha: current_sha,
            is_main,
            is_bare,
            commit_count: None,
        });
    }

    Ok(worktrees)
}

/// Get the count of commits on a branch that are not on main.
/// Useful for showing "3 commits ahead" in the worktree UI.
pub fn commits_ahead_of_main(root_path: &Path, branch: &str) -> Result<usize, String> {
    // Determine main branch name (main or master)
    let main_branch = detect_main_branch(root_path)?;

    let output = std::process::Command::new("git")
        .args(["rev-list", "--count", &format!("{}..{}", main_branch, branch)])
        .current_dir(root_path)
        .output()
        .map_err(|e| format!("Failed to run git rev-list: {}", e))?;

    if !output.status.success() {
        return Ok(0);
    }

    let count_str = String::from_utf8_lossy(&output.stdout).trim().to_string();
    count_str.parse().map_err(|_| "Invalid count".to_string())
}

/// Create a new worktree at `worktree_path` on branch `branch`.
/// If the branch doesn't exist, creates it from HEAD.
pub fn create_worktree(root_path: &Path, branch: &str, worktree_path: &Path) -> Result<WorktreeInfo, String> {
    // Check if worktree path already exists
    if worktree_path.exists() {
        return Err(format!("Worktree path already exists: {}", worktree_path.display()));
    }

    // Check if branch exists
    let repo = git2::Repository::open(root_path)
        .map_err(|e| format!("Failed to open repo: {}", e))?;
    let branch_exists = repo.find_branch(branch, git2::BranchType::Local).is_ok();
    drop(repo);

    let output = if branch_exists {
        std::process::Command::new("git")
            .args(["worktree", "add", &worktree_path.to_string_lossy(), branch])
            .current_dir(root_path)
            .output()
    } else {
        // Create new branch from HEAD
        std::process::Command::new("git")
            .args(["worktree", "add", "-b", branch, &worktree_path.to_string_lossy()])
            .current_dir(root_path)
            .output()
    }
    .map_err(|e| format!("Failed to run git worktree add: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("git worktree add failed: {}", stderr.trim()));
    }

    // Get HEAD SHA of the new worktree
    let head_sha = std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(worktree_path)
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default();

    Ok(WorktreeInfo {
        path: worktree_path.to_string_lossy().to_string(),
        branch: branch.to_string(),
        head_sha,
        is_main: false,
        is_bare: false,
        commit_count: Some(0),
    })
}

/// Remove a worktree by path. Optionally force-removes even if dirty.
/// If `delete_branch` is true, also deletes the associated branch.
pub fn remove_worktree(root_path: &Path, worktree_path: &str, force: bool, delete_branch: bool) -> Result<String, String> {
    // First, figure out the branch name before removing
    let branch_name = if delete_branch {
        let worktrees = list_worktrees(root_path)?;
        worktrees.iter()
            .find(|w| w.path == worktree_path)
            .map(|w| w.branch.clone())
    } else {
        None
    };

    let mut args = vec!["worktree", "remove"];
    if force {
        args.push("--force");
    }
    args.push(worktree_path);

    let output = std::process::Command::new("git")
        .args(&args)
        .current_dir(root_path)
        .output()
        .map_err(|e| format!("Failed to run git worktree remove: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("git worktree remove failed: {}", stderr.trim()));
    }

    // Delete the branch if requested
    if let Some(branch) = branch_name {
        if !branch.is_empty() && branch != "(detached)" {
            let _ = std::process::Command::new("git")
                .args(["branch", "-d", &branch])
                .current_dir(root_path)
                .output();
        }
    }

    Ok(format!("Worktree removed: {}", worktree_path))
}

fn detect_main_branch(root_path: &Path) -> Result<String, String> {
    let repo = git2::Repository::open(root_path)
        .map_err(|e| format!("Failed to open repo: {}", e))?;

    // Check if "main" exists
    if repo.find_branch("main", git2::BranchType::Local).is_ok() {
        return Ok("main".to_string());
    }
    if repo.find_branch("master", git2::BranchType::Local).is_ok() {
        return Ok("master".to_string());
    }
    // Fallback: use HEAD
    Ok("HEAD".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_porcelain_single_worktree() {
        let output = "worktree /home/user/project\nHEAD abc1234def5678\nbranch refs/heads/main\n\n";
        let result = parse_porcelain_worktrees(output, Path::new("/home/user/project")).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].branch, "main");
        assert!(result[0].is_main);
    }

    #[test]
    fn parse_porcelain_multiple_worktrees() {
        let output = "\
worktree /home/user/project
HEAD abc1234
branch refs/heads/main

worktree /home/user/project-feat
HEAD def5678
branch refs/heads/feat/auth

";
        let result = parse_porcelain_worktrees(output, Path::new("/home/user/project")).unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].branch, "main");
        assert!(result[0].is_main);
        assert_eq!(result[1].branch, "feat/auth");
        assert!(!result[1].is_main);
    }
}

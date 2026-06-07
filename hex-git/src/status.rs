//! Git status: current branch, dirty file count, ahead/behind remote.

use std::path::Path;

use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitStatus {
    pub branch: String,
    pub head_sha: String,
    pub is_detached: bool,
    pub dirty_count: usize,
    pub staged_count: usize,
    pub untracked_count: usize,
    pub ahead: usize,
    pub behind: usize,
    pub stash_count: usize,
    pub files: Vec<StatusFile>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StatusFile {
    pub path: String,
    pub status: String, // "modified", "added", "deleted", "renamed", "untracked"
    pub staged: bool,
}

pub fn get_status(root_path: &Path) -> Result<GitStatus, String> {
    let repo = git2::Repository::open(root_path)
        .map_err(|e| format!("Failed to open repo: {}", e))?;

    // Current branch
    let (branch, is_detached) = match repo.head() {
        Ok(head) => {
            if head.is_branch() {
                let name = head.shorthand().unwrap_or("HEAD").to_string();
                (name, false)
            } else {
                ("HEAD".to_string(), true)
            }
        }
        Err(_) => ("(no commits)".to_string(), false),
    };

    // HEAD SHA
    let head_sha = repo
        .head()
        .ok()
        .and_then(|h| h.target())
        .map(|oid| format!("{}", oid))
        .unwrap_or_default();

    // Ahead/behind (compute before statuses to avoid borrow conflicts)
    let (ahead, behind) = compute_ahead_behind(&repo, &branch);

    // Stash count
    let stash_count = {
        let mut count = 0usize;
        // stash_foreach requires &mut self
        let mut repo_mut = git2::Repository::open(root_path)
            .map_err(|e| format!("Failed to reopen repo for stash: {}", e))?;
        let _ = repo_mut.stash_foreach(|_, _, _| {
            count += 1;
            true
        });
        count
    };

    // File statuses
    let mut opts = git2::StatusOptions::new();
    opts.include_untracked(true)
        .recurse_untracked_dirs(true)
        .renames_head_to_index(true);

    let statuses = repo
        .statuses(Some(&mut opts))
        .map_err(|e| format!("Failed to get status: {}", e))?;

    let mut dirty_count = 0usize;
    let mut staged_count = 0usize;
    let mut untracked_count = 0usize;
    let mut files = Vec::new();

    for entry in statuses.iter() {
        let st = entry.status();
        let path = entry.path().unwrap_or("").to_string();

        if st.is_wt_new() {
            untracked_count += 1;
            files.push(StatusFile {
                path,
                status: "untracked".to_string(),
                staged: false,
            });
        } else {
            // Working tree changes
            if st.is_wt_modified() || st.is_wt_deleted() || st.is_wt_renamed() || st.is_wt_typechange() {
                dirty_count += 1;
                let status_str = if st.is_wt_modified() {
                    "modified"
                } else if st.is_wt_deleted() {
                    "deleted"
                } else if st.is_wt_renamed() {
                    "renamed"
                } else {
                    "typechange"
                };
                files.push(StatusFile {
                    path: path.clone(),
                    status: status_str.to_string(),
                    staged: false,
                });
            }
            // Index (staged) changes
            if st.is_index_new()
                || st.is_index_modified()
                || st.is_index_deleted()
                || st.is_index_renamed()
                || st.is_index_typechange()
            {
                staged_count += 1;
                let status_str = if st.is_index_new() {
                    "added"
                } else if st.is_index_modified() {
                    "modified"
                } else if st.is_index_deleted() {
                    "deleted"
                } else if st.is_index_renamed() {
                    "renamed"
                } else {
                    "typechange"
                };
                files.push(StatusFile {
                    path,
                    status: status_str.to_string(),
                    staged: true,
                });
            }
        }
    }

    Ok(GitStatus {
        branch,
        head_sha,
        is_detached,
        dirty_count,
        staged_count,
        untracked_count,
        ahead,
        behind,
        stash_count,
        files,
    })
}

fn compute_ahead_behind(repo: &git2::Repository, branch: &str) -> (usize, usize) {
    let local = match repo.revparse_single(&format!("refs/heads/{}", branch)) {
        Ok(obj) => obj.id(),
        Err(_) => return (0, 0),
    };

    let upstream_ref = format!("refs/remotes/origin/{}", branch);
    let remote = match repo.revparse_single(&upstream_ref) {
        Ok(obj) => obj.id(),
        Err(_) => return (0, 0),
    };

    repo.graph_ahead_behind(local, remote).unwrap_or((0, 0))
}

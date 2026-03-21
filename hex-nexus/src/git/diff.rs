//! Git diff: working tree and inter-ref diffs.
//!
//! Uses `git2::Patch` per-delta to avoid the borrow-checker issues
//! with `Diff::foreach`'s multiple closure captures.

use std::path::Path;

use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiffResult {
    pub files: Vec<DiffFile>,
    pub stats: DiffStats,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiffFile {
    pub path: String,
    pub old_path: Option<String>,
    pub status: String, // "added", "deleted", "modified", "renamed"
    pub additions: usize,
    pub deletions: usize,
    pub hunks: Vec<DiffHunk>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiffHunk {
    pub header: String,
    pub old_start: u32,
    pub old_lines: u32,
    pub new_start: u32,
    pub new_lines: u32,
    pub lines: Vec<DiffLine>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiffLine {
    pub origin: String, // "+", "-", " "
    pub content: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiffStats {
    pub files_changed: usize,
    pub insertions: usize,
    pub deletions: usize,
}

/// Working tree diff (unstaged changes). If `staged` is true, shows index vs HEAD.
pub fn get_working_diff(root_path: &Path, staged: bool) -> Result<DiffResult, String> {
    let repo = git2::Repository::open(root_path)
        .map_err(|e| format!("Failed to open repo: {}", e))?;

    let diff = if staged {
        let head_tree = repo
            .head()
            .ok()
            .and_then(|h| h.peel_to_tree().ok());
        repo.diff_tree_to_index(head_tree.as_ref(), None, None)
    } else {
        repo.diff_index_to_workdir(None, None)
    }
    .map_err(|e| format!("Failed to compute diff: {}", e))?;

    parse_diff(&diff)
}

/// Diff between two refs (e.g., "main" and "feat/auth").
pub fn get_ref_diff(root_path: &Path, base: &str, head: &str) -> Result<DiffResult, String> {
    let repo = git2::Repository::open(root_path)
        .map_err(|e| format!("Failed to open repo: {}", e))?;

    let base_tree = resolve_tree(&repo, base)?;
    let head_tree = resolve_tree(&repo, head)?;

    let diff = repo
        .diff_tree_to_tree(Some(&base_tree), Some(&head_tree), None)
        .map_err(|e| format!("Failed to compute diff: {}", e))?;

    parse_diff(&diff)
}

fn resolve_tree<'r>(repo: &'r git2::Repository, refspec: &str) -> Result<git2::Tree<'r>, String> {
    let obj = repo
        .revparse_single(refspec)
        .map_err(|e| format!("Cannot resolve '{}': {}", refspec, e))?;
    obj.peel_to_tree()
        .map_err(|e| format!("'{}' does not point to a tree: {}", refspec, e))
}

/// Public entry point for parsing a diff (used by commit detail route).
pub fn parse_diff_public(diff: &git2::Diff) -> Result<DiffResult, String> {
    parse_diff(diff)
}

/// Parse a diff using per-delta Patch objects (avoids foreach borrow issues).
fn parse_diff(diff: &git2::Diff) -> Result<DiffResult, String> {
    let num_deltas = diff.deltas().len();
    let mut files = Vec::with_capacity(num_deltas);
    let mut total_insertions = 0usize;
    let mut total_deletions = 0usize;

    for i in 0..num_deltas {
        let delta = diff.get_delta(i).unwrap();
        let new_file = delta.new_file();
        let old_file = delta.old_file();

        let path = new_file
            .path()
            .or_else(|| old_file.path())
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default();

        let old_path = if delta.status() == git2::Delta::Renamed {
            old_file.path().map(|p| p.to_string_lossy().to_string())
        } else {
            None
        };

        let status = match delta.status() {
            git2::Delta::Added => "added",
            git2::Delta::Deleted => "deleted",
            git2::Delta::Modified => "modified",
            git2::Delta::Renamed => "renamed",
            git2::Delta::Copied => "copied",
            _ => "modified",
        };

        // Use Patch to get hunk/line details for this delta
        let mut hunks = Vec::new();
        let mut file_additions = 0usize;
        let mut file_deletions = 0usize;

        if let Ok(Some(patch)) = git2::Patch::from_diff(diff, i) {
            let num_hunks = patch.num_hunks();
            for h in 0..num_hunks {
                if let Ok((hunk, _num_lines_in_hunk)) = patch.hunk(h) {
                    let mut lines = Vec::new();
                    let num_lines = patch.num_lines_in_hunk(h).unwrap_or(0);
                    for l in 0..num_lines {
                        if let Ok(line) = patch.line_in_hunk(h, l) {
                            let origin = match line.origin() {
                                '+' => {
                                    file_additions += 1;
                                    "+"
                                }
                                '-' => {
                                    file_deletions += 1;
                                    "-"
                                }
                                ' ' => " ",
                                _ => " ",
                            };
                            let content = std::str::from_utf8(line.content())
                                .unwrap_or("")
                                .to_string();
                            lines.push(DiffLine {
                                origin: origin.to_string(),
                                content,
                            });
                        }
                    }
                    hunks.push(DiffHunk {
                        header: std::str::from_utf8(hunk.header())
                            .unwrap_or("")
                            .to_string(),
                        old_start: hunk.old_start(),
                        old_lines: hunk.old_lines(),
                        new_start: hunk.new_start(),
                        new_lines: hunk.new_lines(),
                        lines,
                    });
                }
            }
        }

        total_insertions += file_additions;
        total_deletions += file_deletions;

        files.push(DiffFile {
            path,
            old_path,
            status: status.to_string(),
            additions: file_additions,
            deletions: file_deletions,
            hunks,
        });
    }

    Ok(DiffResult {
        stats: DiffStats {
            files_changed: files.len(),
            insertions: total_insertions,
            deletions: total_deletions,
        },
        files,
    })
}

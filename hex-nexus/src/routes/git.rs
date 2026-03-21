//! Git REST API routes (ADR-044 Phase 1+2).
//!
//! All endpoints are project-scoped: `/api/{project_id}/git/...`
//! Git operations run on `spawn_blocking` since git2 does synchronous I/O.

use axum::{
    extract::{Path, Query, State},
    Json,
};
use http::StatusCode;
use serde::Deserialize;
use serde_json::json;

use crate::state::{SharedState, WsEnvelope};

// ── Helpers ────────────────────────────────────────────

/// Resolve a project to its filesystem root_path.
///
/// Strategy (in order):
/// 1. `?path=` query parameter (preferred — frontend passes SpacetimeDB path directly)
/// 2. REST project registry lookup by ID, name, or basename (legacy fallback)
///
/// Architecture: nexus is stateless filesystem I/O. Business logic
/// (project registry) lives in SpacetimeDB. The frontend reads the path
/// from SpacetimeDB and passes it via `?path=`.
async fn resolve_project_path(state: &SharedState, project_id: &str) -> Result<String, (StatusCode, Json<serde_json::Value>)> {
    let sp = state.state_port.as_ref().ok_or_else(|| (
        StatusCode::SERVICE_UNAVAILABLE,
        Json(json!({ "ok": false, "error": "State port not configured" })),
    ))?;

    // Use project_find which checks by ID, name, and basename
    match sp.project_find(project_id).await {
        Ok(Some(p)) => Ok(p.root_path),
        Ok(None) => Err((
            StatusCode::NOT_FOUND,
            Json(json!({ "ok": false, "error": format!("Project '{}' not found. Register via POST /api/projects/register with rootPath", project_id) })),
        )),
        Err(e) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "ok": false, "error": e.to_string() })),
        )),
    }
}

fn git_error(msg: String) -> (StatusCode, Json<serde_json::Value>) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({ "ok": false, "error": msg })),
    )
}

// ── GET /api/{project_id}/git/status ───────────────────

pub async fn git_status(
    State(state): State<SharedState>,
    Path(project_id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let root_path = resolve_project_path(&state, &project_id).await?;

    let result = tokio::task::spawn_blocking(move || {
        crate::git::status::get_status(std::path::Path::new(&root_path))
    })
    .await
    .map_err(|e| git_error(format!("Task join error: {}", e)))?
    .map_err(|e| git_error(e))?;

    Ok(Json(json!({ "ok": true, "data": result })))
}

// ── GET /api/{project_id}/git/log ──────────────────────

#[derive(Debug, Deserialize)]
pub struct LogQuery {
    pub branch: Option<String>,
    pub cursor: Option<String>,
    pub limit: Option<usize>,
}

pub async fn git_log(
    State(state): State<SharedState>,
    Path(project_id): Path<String>,
    Query(q): Query<LogQuery>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let root_path = resolve_project_path(&state, &project_id).await?;
    let limit = q.limit.unwrap_or(20).min(100);

    let result = tokio::task::spawn_blocking(move || {
        crate::git::log::get_log(
            std::path::Path::new(&root_path),
            q.branch.as_deref(),
            q.cursor.as_deref(),
            limit,
        )
    })
    .await
    .map_err(|e| git_error(format!("Task join error: {}", e)))?
    .map_err(|e| git_error(e))?;

    Ok(Json(json!({ "ok": true, "data": result })))
}

// ── GET /api/{project_id}/git/diff ─────────────────────

#[derive(Debug, Deserialize)]
pub struct DiffQuery {
    pub staged: Option<bool>,
}

pub async fn git_diff(
    State(state): State<SharedState>,
    Path(project_id): Path<String>,
    Query(q): Query<DiffQuery>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let root_path = resolve_project_path(&state, &project_id).await?;
    let staged = q.staged.unwrap_or(false);

    let result = tokio::task::spawn_blocking(move || {
        crate::git::diff::get_working_diff(std::path::Path::new(&root_path), staged)
    })
    .await
    .map_err(|e| git_error(format!("Task join error: {}", e)))?
    .map_err(|e| git_error(e))?;

    Ok(Json(json!({ "ok": true, "data": result })))
}

// ── GET /api/{project_id}/git/diff/{base}...{head} ─────

pub async fn git_diff_refs(
    State(state): State<SharedState>,
    Path((project_id, refspec)): Path<(String, String)>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let root_path = resolve_project_path(&state, &project_id).await?;

    // Parse "base...head" format
    let parts: Vec<&str> = refspec.splitn(2, "...").collect();
    if parts.len() != 2 {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(json!({ "ok": false, "error": "Expected format: base...head" })),
        ));
    }
    let base = parts[0].to_string();
    let head = parts[1].to_string();

    let result = tokio::task::spawn_blocking(move || {
        crate::git::diff::get_ref_diff(std::path::Path::new(&root_path), &base, &head)
    })
    .await
    .map_err(|e| git_error(format!("Task join error: {}", e)))?
    .map_err(|e| git_error(e))?;

    Ok(Json(json!({ "ok": true, "data": result })))
}

// ── GET /api/{project_id}/git/branches ─────────────────

pub async fn git_branches(
    State(state): State<SharedState>,
    Path(project_id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let root_path = resolve_project_path(&state, &project_id).await?;

    let result = tokio::task::spawn_blocking(move || {
        get_branches(std::path::Path::new(&root_path))
    })
    .await
    .map_err(|e| git_error(format!("Task join error: {}", e)))?
    .map_err(|e| git_error(e))?;

    Ok(Json(json!({ "ok": true, "data": result })))
}

fn get_branches(root_path: &std::path::Path) -> Result<serde_json::Value, String> {
    let repo = git2::Repository::open(root_path)
        .map_err(|e| format!("Failed to open repo: {}", e))?;

    let mut branches = Vec::new();

    let branch_iter = repo
        .branches(None)
        .map_err(|e| format!("Failed to list branches: {}", e))?;

    for branch_result in branch_iter {
        let (branch, branch_type) = branch_result
            .map_err(|e| format!("Branch iteration error: {}", e))?;

        let name = branch.name()
            .map_err(|e| format!("Invalid branch name: {}", e))?
            .unwrap_or("")
            .to_string();

        let sha = branch
            .get()
            .target()
            .map(|oid| format!("{}", oid))
            .unwrap_or_default();

        let is_remote = matches!(branch_type, git2::BranchType::Remote);
        let is_head = branch.is_head();

        branches.push(json!({
            "name": name,
            "sha": sha,
            "shortSha": &sha[..7.min(sha.len())],
            "isRemote": is_remote,
            "isHead": is_head,
        }));
    }

    Ok(json!({ "branches": branches }))
}

// ── GET /api/{project_id}/git/worktrees ────────────────

pub async fn git_worktrees(
    State(state): State<SharedState>,
    Path(project_id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let root_path = resolve_project_path(&state, &project_id).await?;
    let root_clone = root_path.clone();

    let mut worktrees = tokio::task::spawn_blocking(move || {
        crate::git::worktree::list_worktrees(std::path::Path::new(&root_path))
    })
    .await
    .map_err(|e| git_error(format!("Task join error: {}", e)))?
    .map_err(|e| git_error(e))?;

    // Enrich non-main worktrees with commit count ahead of main
    let root_for_count = root_clone;
    for wt in &mut worktrees {
        if !wt.is_main && !wt.branch.is_empty() && wt.branch != "(detached)" {
            let branch = wt.branch.clone();
            let rp = root_for_count.clone();
            if let Ok(count) = crate::git::worktree::commits_ahead_of_main(
                std::path::Path::new(&rp),
                &branch,
            ) {
                wt.commit_count = Some(count);
            }
        }
    }

    Ok(Json(json!({ "ok": true, "data": { "worktrees": worktrees } })))
}

// ── POST /api/{project_id}/git/worktrees ───────────────
// Phase 2: Create a new worktree

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateWorktreeRequest {
    pub branch: String,
    pub path: Option<String>, // Optional custom path; defaults to sibling directory
}

pub async fn git_worktree_create(
    State(state): State<SharedState>,
    Path(project_id): Path<String>,
    Json(req): Json<CreateWorktreeRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let root_path = resolve_project_path(&state, &project_id).await?;
    let ws_tx = state.ws_tx.clone();

    let branch = req.branch.clone();
    let custom_path = req.path.clone();

    let result = tokio::task::spawn_blocking(move || {
        let root = std::path::Path::new(&root_path);

        // Determine worktree path: custom or auto-generated sibling
        let wt_path = if let Some(p) = custom_path {
            std::path::PathBuf::from(p)
        } else {
            // Default: sibling directory named after branch (slashes → dashes)
            let safe_name = branch.replace('/', "-");
            let parent = root.parent().unwrap_or(root);
            parent.join(format!("{}-{}", root.file_name().unwrap_or_default().to_string_lossy(), safe_name))
        };

        crate::git::worktree::create_worktree(root, &branch, &wt_path)
    })
    .await
    .map_err(|e| git_error(format!("Task join error: {}", e)))?
    .map_err(|e| git_error(e))?;

    // Broadcast worktree creation via WebSocket
    let _ = ws_tx.send(WsEnvelope {
        topic: format!("project:{}:git", project_id),
        event: "worktree-created".to_string(),
        data: json!({
            "branch": result.branch,
            "path": result.path,
        }),
    });

    Ok(Json(json!({ "ok": true, "data": result })))
}

// ── DELETE /api/{project_id}/git/worktrees/{name} ──────
// Phase 2: Remove a worktree

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoveWorktreeQuery {
    pub force: Option<bool>,
    pub delete_branch: Option<bool>,
}

pub async fn git_worktree_delete(
    State(state): State<SharedState>,
    Path((project_id, worktree_name)): Path<(String, String)>,
    Query(q): Query<RemoveWorktreeQuery>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let root_path = resolve_project_path(&state, &project_id).await?;
    let ws_tx = state.ws_tx.clone();
    let force = q.force.unwrap_or(false);
    let delete_branch = q.delete_branch.unwrap_or(false);

    // Resolve worktree_name to actual path by listing worktrees
    let root_clone = root_path.clone();
    let name_clone = worktree_name.clone();

    let result = tokio::task::spawn_blocking(move || {
        let root = std::path::Path::new(&root_clone);

        // Try to find the worktree by branch name or path suffix
        let worktrees = crate::git::worktree::list_worktrees(root)?;
        let target = worktrees.iter().find(|w| {
            w.branch == name_clone
                || w.path.ends_with(&name_clone)
                || w.path == name_clone
        });

        match target {
            Some(wt) => {
                if wt.is_main {
                    return Err("Cannot remove the main worktree".to_string());
                }
                crate::git::worktree::remove_worktree(root, &wt.path, force, delete_branch)
            }
            None => Err(format!("Worktree '{}' not found", name_clone)),
        }
    })
    .await
    .map_err(|e| git_error(format!("Task join error: {}", e)))?
    .map_err(|e| git_error(e))?;

    // Broadcast worktree removal via WebSocket
    let _ = ws_tx.send(WsEnvelope {
        topic: format!("project:{}:git", project_id),
        event: "worktree-removed".to_string(),
        data: json!({ "name": worktree_name }),
    });

    Ok(Json(json!({ "ok": true, "message": result })))
}

// ── GET /api/{project_id}/git/log/{sha} ────────────────
// Phase 2: Single commit detail with diff

pub async fn git_commit_detail(
    State(state): State<SharedState>,
    Path((project_id, sha)): Path<(String, String)>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let root_path = resolve_project_path(&state, &project_id).await?;

    let result = tokio::task::spawn_blocking(move || {
        let root = std::path::Path::new(&root_path);
        let repo = git2::Repository::open(root)
            .map_err(|e| format!("Failed to open repo: {}", e))?;

        let oid = git2::Oid::from_str(&sha)
            .map_err(|e| format!("Invalid SHA: {}", e))?;
        let commit = repo.find_commit(oid)
            .map_err(|e| format!("Commit not found: {}", e))?;

        let commit_info = crate::git::log::CommitInfo {
            sha: format!("{}", oid),
            short_sha: format!("{}", oid)[..7].to_string(),
            message: commit.message().unwrap_or("").to_string(),
            author_name: commit.author().name().unwrap_or("").to_string(),
            author_email: commit.author().email().unwrap_or("").to_string(),
            timestamp: commit.time().seconds(),
            parent_count: commit.parent_count(),
        };

        // Compute diff: commit tree vs first parent tree
        let commit_tree = commit.tree()
            .map_err(|e| format!("Cannot get commit tree: {}", e))?;
        let parent_tree = if commit.parent_count() > 0 {
            commit.parent(0).ok().and_then(|p| p.tree().ok())
        } else {
            None
        };

        let diff = repo.diff_tree_to_tree(parent_tree.as_ref(), Some(&commit_tree), None)
            .map_err(|e| format!("Failed to compute commit diff: {}", e))?;

        let diff_result = crate::git::diff::parse_diff_public(&diff)?;

        Ok::<_, String>(json!({
            "commit": commit_info,
            "diff": diff_result,
        }))
    })
    .await
    .map_err(|e| git_error(format!("Task join error: {}", e)))?
    .map_err(|e| git_error(e))?;

    Ok(Json(json!({ "ok": true, "data": result })))
}

// ═══════════════════════════════════════════════════════════
// PHASE 3: Cross-cutting git intelligence
// ═══════════════════════════════════════════════════════════

// ── GET /api/{project_id}/git/task-commits ──────────────
// Scan commit messages for task IDs and agent attribution

#[derive(Debug, Deserialize)]
pub struct TaskCommitsQuery {
    pub limit: Option<usize>,
}

pub async fn git_task_commits(
    State(state): State<SharedState>,
    Path(project_id): Path<String>,
    Query(q): Query<TaskCommitsQuery>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let root_path = resolve_project_path(&state, &project_id).await?;
    let limit = q.limit.unwrap_or(50).min(200);

    let result = tokio::task::spawn_blocking(move || {
        crate::git::correlation::find_task_commits(std::path::Path::new(&root_path), limit)
    })
    .await
    .map_err(|e| git_error(format!("Task join error: {}", e)))?
    .map_err(|e| git_error(e))?;

    Ok(Json(json!({ "ok": true, "data": { "links": result } })))
}

// ── POST /api/{project_id}/git/violation-blame ─────────
// Blame architecture violations to find which commit introduced them

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ViolationBlameRequest {
    pub violations: Vec<ViolationInputDto>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ViolationInputDto {
    pub file: String,
    pub line: usize,
    pub message: String,
}

pub async fn git_violation_blame(
    State(state): State<SharedState>,
    Path(project_id): Path<String>,
    Json(req): Json<ViolationBlameRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let root_path = resolve_project_path(&state, &project_id).await?;

    let violations: Vec<crate::git::blame::ViolationInput> = req
        .violations
        .into_iter()
        .map(|v| crate::git::blame::ViolationInput {
            file: v.file,
            line: v.line,
            message: v.message,
        })
        .collect();

    let result = tokio::task::spawn_blocking(move || {
        crate::git::blame::blame_violations(std::path::Path::new(&root_path), &violations)
    })
    .await
    .map_err(|e| git_error(format!("Task join error: {}", e)))?;

    Ok(Json(json!({ "ok": true, "data": { "blames": result } })))
}

// ── GET /api/{project_id}/git/timeline ─────────────────
// Unified timeline merging git commits with HexFlo task events

#[derive(Debug, Deserialize)]
pub struct TimelineQuery {
    pub limit: Option<usize>,
}

pub async fn git_timeline(
    State(state): State<SharedState>,
    Path(project_id): Path<String>,
    Query(q): Query<TimelineQuery>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let root_path = resolve_project_path(&state, &project_id).await?;
    let limit = q.limit.unwrap_or(30).min(100);

    // Fetch HexFlo tasks for this project's swarms
    let tasks = if let Some(ref hexflo) = state.hexflo {
        let filter = crate::coordination::TaskFilter {
            swarm_id: None,
            status: None,
        };
        hexflo.task_list(filter).await.unwrap_or_default()
    } else {
        Vec::new()
    };

    let result = tokio::task::spawn_blocking(move || {
        crate::git::timeline::build_timeline(std::path::Path::new(&root_path), &tasks, limit)
    })
    .await
    .map_err(|e| git_error(format!("Task join error: {}", e)))?
    .map_err(|e| git_error(e))?;

    Ok(Json(json!({ "ok": true, "data": { "entries": result } })))
}

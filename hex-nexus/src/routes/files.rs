use axum::Json;
use axum::extract::Query;
use http::StatusCode;
use serde::Deserialize;
use serde_json::json;
use std::path::{Path, PathBuf};

#[derive(Debug, Deserialize)]
pub struct SaveFileRequest {
    pub path: String,
    pub content: String,
}

/// Resolve the project root directory.
fn find_project_root() -> Option<PathBuf> {
    // Prefer explicit env var
    if let Ok(root) = std::env::var("HEX_PROJECT_ROOT") {
        let p = PathBuf::from(&root);
        if p.is_dir() {
            return Some(p);
        }
    }

    // Walk up from cwd looking for a CLAUDE.md or .git directory
    if let Ok(cwd) = std::env::current_dir() {
        let mut dir = cwd.as_path();
        loop {
            if dir.join("CLAUDE.md").exists() || dir.join(".git").exists() {
                return Some(dir.to_path_buf());
            }
            match dir.parent() {
                Some(p) => dir = p,
                None => break,
            }
        }
        // Fallback: use cwd itself
        return Some(cwd);
    }

    None
}

/// Validate that a relative path does not escape the project root.
/// Rejects paths containing `..`, absolute paths, and null bytes.
fn is_safe_path(relative: &str) -> bool {
    if relative.contains('\0') {
        return false;
    }

    let path = Path::new(relative);

    // Reject absolute paths
    if path.is_absolute() {
        return false;
    }

    // Reject any component that is `..`
    for component in path.components() {
        match component {
            std::path::Component::ParentDir => return false,
            _ => {}
        }
    }

    true
}

#[derive(Debug, Deserialize)]
pub struct ReadFileParams {
    pub path: String,
    /// When "true", list directory contents instead of reading file content.
    pub list: Option<String>,
}

/// GET /api/files?path=X — read a file or list a directory (path-traversal protected).
///
/// Query params:
///   - `path` (required): relative path within the project root
///   - `list=true` (optional): if the path is a directory, return a JSON array of filenames
///
/// Returns:
///   - For files: `{ "content": "..." }`
///   - For directories (list=true): `{ "files": ["a.md", "b.md"] }`
pub async fn read_file(
    Query(params): Query<ReadFileParams>,
) -> (StatusCode, Json<serde_json::Value>) {
    if !is_safe_path(&params.path) {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "Invalid path: must be relative and must not contain '..'" })),
        );
    }

    let root = match find_project_root() {
        Some(r) => r,
        None => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "Could not determine project root" })),
            );
        }
    };

    let target = root.join(&params.path);

    // Canonicalize to prevent traversal via symlinks
    if target.exists() {
        let canonical_root = root.canonicalize().unwrap_or(root.clone());
        let canonical_target = target.canonicalize().unwrap_or(target.clone());
        if !canonical_target.starts_with(&canonical_root) {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": "Path escapes project root" })),
            );
        }
    }

    let is_list = params.list.as_deref() == Some("true");

    if target.is_dir() {
        if !is_list {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": "Path is a directory. Use list=true to list contents." })),
            );
        }
        match std::fs::read_dir(&target) {
            Ok(entries) => {
                let files: Vec<String> = entries
                    .filter_map(|e| e.ok())
                    .map(|e| e.file_name().to_string_lossy().to_string())
                    .collect();
                (StatusCode::OK, Json(json!({ "files": files })))
            }
            Err(e) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": format!("Failed to read directory: {}", e) })),
            ),
        }
    } else if target.is_file() {
        match std::fs::read_to_string(&target) {
            Ok(content) => (StatusCode::OK, Json(json!({ "content": content }))),
            Err(e) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": format!("Failed to read file: {}", e) })),
            ),
        }
    } else {
        (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": format!("Path not found: {}", params.path) })),
        )
    }
}

/// PUT /api/files — write content to a project file (path-traversal protected).
pub async fn save_file(
    Json(body): Json<SaveFileRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    // Validate path safety
    if !is_safe_path(&body.path) {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "Invalid path: must be relative and must not contain '..'" })),
        );
    }

    let root = match find_project_root() {
        Some(r) => r,
        None => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "Could not determine project root" })),
            )
        }
    };

    let target = root.join(&body.path);

    // Double-check the resolved path is still under project root
    let canonical_root = root.canonicalize().unwrap_or(root.clone());
    // For new files that don't exist yet, canonicalize the parent
    let target_parent = target.parent().unwrap_or(&target);
    if target_parent.exists() {
        let canonical_target_parent = target_parent.canonicalize().unwrap_or(target_parent.to_path_buf());
        if !canonical_target_parent.starts_with(&canonical_root) {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": "Path escapes project root" })),
            );
        }
    }

    // Ensure parent directory exists
    if let Some(parent) = target.parent() {
        if !parent.exists() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({ "error": format!("Failed to create directory: {}", e) })),
                );
            }
        }
    }

    match std::fs::write(&target, &body.content) {
        Ok(()) => (
            StatusCode::OK,
            Json(json!({ "ok": true, "path": body.path })),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": format!("Failed to write file: {}", e) })),
        ),
    }
}

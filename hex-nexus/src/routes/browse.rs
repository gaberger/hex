use axum::{
    extract::{Path, Query, State},
    Json,
};
use http::StatusCode;
use serde::Deserialize;
use serde_json::json;
use std::path::PathBuf;

use crate::state::SharedState;

const MAX_ENTRIES: usize = 500;
const MAX_FILE_SIZE: u64 = 512 * 1024; // 512KB

/// Hidden directories to skip in listings.
const HIDDEN_DIRS: &[&str] = &[
    ".git", "node_modules", ".next", "__pycache__", "target",
    ".cache", ".vscode", ".idea", "dist", "build",
];

/// Resolve `relative` under `root`, rejecting any path traversal.
fn safe_resolve(root: &str, relative: &str) -> Result<PathBuf, &'static str> {
    let root_canon = std::fs::canonicalize(root).map_err(|_| "root not found")?;
    let candidate = root_canon.join(relative);
    let candidate_canon = std::fs::canonicalize(&candidate).map_err(|_| "path not found")?;
    if !candidate_canon.starts_with(&root_canon) {
        return Err("path traversal rejected");
    }
    // Reject symlinks that escape root
    if candidate.is_symlink() {
        let target = std::fs::read_link(&candidate).map_err(|_| "symlink unreadable")?;
        let resolved = if target.is_absolute() {
            target
        } else {
            candidate.parent().unwrap_or(&root_canon).join(&target)
        };
        let resolved_canon = std::fs::canonicalize(&resolved).map_err(|_| "symlink target not found")?;
        if !resolved_canon.starts_with(&root_canon) {
            return Err("symlink escapes root");
        }
    }
    Ok(candidate_canon)
}

/// Infer language from file extension for syntax highlighting.
fn infer_language(path: &std::path::Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()) {
        Some("rs") => "rust",
        Some("ts") | Some("tsx") => "typescript",
        Some("js") | Some("jsx") | Some("mjs") | Some("cjs") => "javascript",
        Some("json") => "json",
        Some("toml") => "toml",
        Some("yaml") | Some("yml") => "yaml",
        Some("md") | Some("markdown") => "markdown",
        Some("html") | Some("htm") => "html",
        Some("css") => "css",
        Some("py") => "python",
        Some("sh") | Some("bash") | Some("zsh") => "bash",
        Some("sql") => "sql",
        Some("go") => "go",
        Some("java") => "java",
        Some("c") | Some("h") => "c",
        Some("cpp") | Some("hpp") | Some("cc") => "cpp",
        Some("rb") => "ruby",
        Some("xml") => "xml",
        Some("lock") => "text",
        _ => "text",
    }
}

/// Check if content looks binary (null bytes in first 8KB).
fn is_binary(data: &[u8]) -> bool {
    let check_len = data.len().min(8192);
    data[..check_len].contains(&0)
}

#[derive(Deserialize)]
pub struct BrowseQuery {
    #[serde(default)]
    pub path: String,
}

/// `GET /api/{project_id}/browse?path=` — list directory contents.
pub async fn browse_dir(
    State(state): State<SharedState>,
    Path(project_id): Path<String>,
    Query(query): Query<BrowseQuery>,
) -> (StatusCode, Json<serde_json::Value>) {
    let root_path = match state.state_port.as_ref() {
        Some(sp) => match sp.project_get(&project_id).await {
            Ok(Some(entry)) => entry.root_path,
            Ok(None) => return (StatusCode::NOT_FOUND, Json(json!({ "error": "Project not found" }))),
            Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e.to_string() }))),
        },
        None => return (StatusCode::SERVICE_UNAVAILABLE, Json(json!({ "error": "State port not configured" }))),
    };

    let dir_path = match safe_resolve(&root_path, &query.path) {
        Ok(p) => p,
        Err(e) => return (StatusCode::FORBIDDEN, Json(json!({ "error": e }))),
    };

    if !dir_path.is_dir() {
        return (StatusCode::BAD_REQUEST, Json(json!({ "error": "Not a directory" })));
    }

    let mut entries = Vec::new();
    let mut read_dir = match tokio::fs::read_dir(&dir_path).await {
        Ok(rd) => rd,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e.to_string() }))),
    };

    while let Ok(Some(entry)) = read_dir.next_entry().await {
        if entries.len() >= MAX_ENTRIES {
            break;
        }
        let name = entry.file_name().to_string_lossy().into_owned();

        // Skip hidden files/dirs
        if name.starts_with('.') && name != "." {
            continue;
        }

        let meta = match entry.metadata().await {
            Ok(m) => m,
            Err(_) => continue,
        };

        let kind = if meta.is_dir() { "dir" } else { "file" };

        // Skip blacklisted directories
        if meta.is_dir() && HIDDEN_DIRS.contains(&name.as_str()) {
            continue;
        }

        entries.push(json!({
            "name": name,
            "kind": kind,
            "size": if meta.is_file() { meta.len() } else { 0 },
        }));
    }

    // Sort: dirs first, then alphabetical
    entries.sort_by(|a, b| {
        let a_kind = a["kind"].as_str().unwrap_or("");
        let b_kind = b["kind"].as_str().unwrap_or("");
        let a_name = a["name"].as_str().unwrap_or("");
        let b_name = b["name"].as_str().unwrap_or("");
        a_kind.cmp(b_kind).reverse().then(a_name.to_lowercase().cmp(&b_name.to_lowercase()))
    });

    (StatusCode::OK, Json(json!({ "entries": entries, "path": query.path })))
}

/// `GET /api/{project_id}/read/{*path}` — read file content.
pub async fn read_file(
    State(state): State<SharedState>,
    Path((project_id, file_path)): Path<(String, String)>,
) -> (StatusCode, Json<serde_json::Value>) {
    let root_path = match state.state_port.as_ref() {
        Some(sp) => match sp.project_get(&project_id).await {
            Ok(Some(entry)) => entry.root_path,
            Ok(None) => return (StatusCode::NOT_FOUND, Json(json!({ "error": "Project not found" }))),
            Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e.to_string() }))),
        },
        None => return (StatusCode::SERVICE_UNAVAILABLE, Json(json!({ "error": "State port not configured" }))),
    };

    let resolved = match safe_resolve(&root_path, &file_path) {
        Ok(p) => p,
        Err(e) => return (StatusCode::FORBIDDEN, Json(json!({ "error": e }))),
    };

    if !resolved.is_file() {
        return (StatusCode::BAD_REQUEST, Json(json!({ "error": "Not a file" })));
    }

    // Check file size
    let meta = match tokio::fs::metadata(&resolved).await {
        Ok(m) => m,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e.to_string() }))),
    };

    if meta.len() > MAX_FILE_SIZE {
        return (StatusCode::PAYLOAD_TOO_LARGE, Json(json!({
            "error": "File too large",
            "size": meta.len(),
            "limit": MAX_FILE_SIZE,
        })));
    }

    let content_bytes = match tokio::fs::read(&resolved).await {
        Ok(c) => c,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e.to_string() }))),
    };

    if is_binary(&content_bytes) {
        return (StatusCode::UNSUPPORTED_MEDIA_TYPE, Json(json!({ "error": "Binary file" })));
    }

    let content = String::from_utf8_lossy(&content_bytes).into_owned();
    let language = infer_language(&resolved);

    (StatusCode::OK, Json(json!({
        "path": file_path,
        "content": content,
        "size": meta.len(),
        "language": language,
    })))
}

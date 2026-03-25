use axum::{extract::{Path, Query, State}, Json};
use http::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::path::{Path as StdPath, PathBuf};

use crate::state::SharedState;

#[derive(Debug, Serialize)]
pub struct ADRSummary {
    pub id: String,
    pub title: String,
    pub status: String,
    pub date: String,
    pub filename: String,
}

#[derive(Debug, Serialize)]
pub struct ADRDetail {
    pub id: String,
    pub title: String,
    pub status: String,
    pub date: String,
    pub content: String,
}

/// Find the ADR directory by walking upward from CWD, matching the CLI's strategy.
fn find_adr_dir() -> Option<PathBuf> {
    // HEX_PROJECT_ROOT takes precedence when explicitly set
    if let Ok(root) = std::env::var("HEX_PROJECT_ROOT") {
        let p = PathBuf::from(root).join("docs/adrs");
        if p.is_dir() {
            return Some(p);
        }
    }
    // Walk upward from CWD — same logic as hex-cli's find_adr_dir()
    let cwd = std::env::current_dir().ok()?;
    let mut dir = cwd.as_path();
    loop {
        let candidate = dir.join("docs").join("adrs");
        if candidate.is_dir() {
            return Some(candidate);
        }
        dir = dir.parent()?;
    }
}

/// Parse frontmatter from ADR markdown to extract title, status, date.
///
/// ADRs use `**Status:** Accepted` style (not YAML frontmatter).
fn parse_adr_frontmatter(content: &str, filename: &str) -> (String, String, String) {
    let mut status = "unknown".to_string();
    let mut date = String::new();
    let mut title = String::new();

    // Extract title from first `# ` heading
    for line in content.lines() {
        if line.starts_with("# ") {
            title = line.trim_start_matches("# ").to_string();
            // Strip leading "ADR-NNN: " prefix from the title text
            if let Some(pos) = title.find(": ") {
                title = title[pos + 2..].to_string();
            }
            break;
        }
    }

    // Extract status and date from bold-label lines
    for line in content.lines() {
        let lower = line.to_lowercase();
        if lower.starts_with("**status:**") || lower.starts_with("status:") {
            status = line
                .split(':')
                .nth(1)
                .unwrap_or("unknown")
                .trim()
                .trim_matches('*')
                .trim()
                .to_string();
        }
        if lower.starts_with("**date:**") || lower.starts_with("date:") {
            date = line
                .split(':')
                .skip(1)
                .collect::<Vec<_>>()
                .join(":")
                .trim()
                .trim_matches('*')
                .trim()
                .to_string();
        }
    }

    // Fallback: derive title from filename slug
    if title.is_empty() {
        title = filename
            .trim_start_matches("ADR-")
            .trim_start_matches("adr-")
            .split('-')
            .skip(1) // skip the number
            .collect::<Vec<_>>()
            .join(" ")
            .trim_end_matches(".md")
            .to_string();
    }

    (title, status, date)
}

/// Extract the full ADR ID from a filename stem, e.g. "ADR-059-foo.md" → "ADR-059",
/// "ADR-2603221500-foo.md" → "ADR-2603221500". Matches CLI's extract_adr_id().
fn extract_id(filename: &str) -> String {
    let stem = filename.trim_end_matches(".md");
    if let Some(rest) = stem.strip_prefix("ADR-").or_else(|| stem.strip_prefix("adr-")) {
        let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
        if !digits.is_empty() {
            return format!("ADR-{}", digits);
        }
    }
    stem.to_string()
}

// ── Shared helpers for directory-based ADR resolution ─────

/// List all ADRs from a given directory. Matches CLI's collect_adrs() behaviour:
/// - Only files starting with "ADR-" (skips TEMPLATE.md, README.md, etc.)
/// - Sorted ascending by filename (same as CLI's path sort)
/// - Full "ADR-NNN" ID format (not bare numeric)
fn list_adrs_from_dir(dir: &StdPath) -> Vec<ADRSummary> {
    let mut adrs: Vec<ADRSummary> = Vec::new();

    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let filename = entry.file_name().to_string_lossy().to_string();
            if !filename.ends_with(".md") || !filename.starts_with("ADR-") {
                continue;
            }

            let id = extract_id(&filename);
            let content = std::fs::read_to_string(entry.path()).unwrap_or_default();
            let (title, status, date) = parse_adr_frontmatter(&content, &filename);

            adrs.push(ADRSummary {
                id,
                title,
                status,
                date,
                filename,
            });
        }
    }

    // Sort ascending by filename — matches CLI sort (a.path.cmp(&b.path))
    adrs.sort_by(|a, b| a.filename.cmp(&b.filename));
    adrs
}

/// Get a single ADR's full content from a given directory.
fn get_adr_from_dir(dir: &StdPath, id: &str) -> Option<ADRDetail> {
    let entries = std::fs::read_dir(dir).ok()?;
    for entry in entries.flatten() {
        let filename = entry.file_name().to_string_lossy().to_string();
        if !filename.ends_with(".md") {
            continue;
        }

        let file_id = extract_id(&filename);
        if file_id == id {
            let content = std::fs::read_to_string(entry.path()).unwrap_or_default();
            let (title, status, date) = parse_adr_frontmatter(&content, &filename);
            return Some(ADRDetail {
                id: id.to_string(),
                title,
                status,
                date,
                content,
            });
        }
    }
    None
}

/// Save ADR content to a file in the given directory, matching by ID.
fn save_adr_in_dir(dir: &StdPath, id: &str, content: &str) -> Result<String, String> {
    let entries = std::fs::read_dir(dir).map_err(|e| format!("Cannot read dir: {}", e))?;
    for entry in entries.flatten() {
        let filename = entry.file_name().to_string_lossy().to_string();
        if !filename.ends_with(".md") {
            continue;
        }

        let file_id = extract_id(&filename);
        if file_id == id {
            std::fs::write(entry.path(), content)
                .map_err(|e| format!("Failed to write ADR: {}", e))?;
            return Ok(filename);
        }
    }
    Err(format!("ADR-{} not found", id))
}

/// Resolve a project to its filesystem root_path via the state port.
async fn resolve_project_path(state: &SharedState, project_id: &str) -> Result<String, (StatusCode, Json<serde_json::Value>)> {
    let sp = state.state_port.as_ref().ok_or_else(|| (
        StatusCode::SERVICE_UNAVAILABLE,
        Json(json!({ "ok": false, "error": "State port not configured" })),
    ))?;

    match sp.project_find(project_id).await {
        Ok(Some(p)) => Ok(p.root_path),
        Ok(None) => Err((
            StatusCode::NOT_FOUND,
            Json(json!({ "ok": false, "error": format!("Project '{}' not found", project_id) })),
        )),
        Err(e) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "ok": false, "error": e.to_string() })),
        )),
    }
}

// ── Global ADR handlers (existing, now delegate to shared fns) ───

#[derive(Deserialize)]
pub struct AdrListQuery {
    status: Option<String>,
}

/// GET /api/adrs — list all ADRs with metadata.
/// Optional `?status=accepted` filter matches CLI's behaviour.
pub async fn list_adrs(
    Query(params): Query<AdrListQuery>,
) -> (StatusCode, Json<serde_json::Value>) {
    let dir = match find_adr_dir() {
        Some(d) => d,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(json!({ "error": "ADR directory not found (docs/adrs/ not found in any parent directory)" })),
            )
        }
    };

    let mut adrs = list_adrs_from_dir(&dir);

    if let Some(filter) = params.status.as_deref().filter(|s| !s.is_empty()) {
        let filter_lower = filter.to_lowercase();
        adrs.retain(|a| a.status.to_lowercase() == filter_lower);
    }

    (
        StatusCode::OK,
        Json(serde_json::to_value(&adrs).unwrap_or_default()),
    )
}

/// GET /api/adrs/:id — get a single ADR's full content.
pub async fn get_adr(Path(id): Path<String>) -> (StatusCode, Json<serde_json::Value>) {
    let dir = match find_adr_dir() {
        Some(d) => d,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(json!({ "error": "ADR directory not found" })),
            )
        }
    };

    match get_adr_from_dir(&dir, &id) {
        Some(detail) => (
            StatusCode::OK,
            Json(serde_json::to_value(&detail).unwrap_or_default()),
        ),
        None => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": format!("ADR-{} not found", id) })),
        ),
    }
}

#[derive(Debug, Deserialize)]
pub struct SaveADRRequest {
    pub content: String,
}

/// PUT /api/adrs/:id — save ADR content back to filesystem.
pub async fn save_adr(
    Path(id): Path<String>,
    Json(body): Json<SaveADRRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    let dir = match find_adr_dir() {
        Some(d) => d,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(json!({ "error": "ADR directory not found" })),
            )
        }
    };

    match save_adr_in_dir(&dir, &id, &body.content) {
        Ok(filename) => (
            StatusCode::OK,
            Json(json!({ "ok": true, "file": filename })),
        ),
        Err(msg) => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": msg })),
        ),
    }
}

// ── Project-scoped ADR handlers (ADR-045 Phase 1) ────────

/// GET /api/projects/{id}/adrs — list ADRs from a project's docs/adrs/ directory.
/// Accepts optional `?root=/abs/path` query param as fallback when project lookup fails.
pub async fn list_project_adrs(
    Path(project_id): Path<String>,
    Query(params): Query<std::collections::HashMap<String, String>>,
    State(state): State<SharedState>,
) -> (StatusCode, Json<serde_json::Value>) {
    let root_path = match resolve_project_path(&state, &project_id).await {
        Ok(p) => p,
        Err(_) => {
            // Fallback: use ?root= query param if provided (dashboard passes project path)
            match params.get("root") {
                Some(r) if !r.is_empty() && std::path::Path::new(r).is_dir() => r.clone(),
                _ => return (
                    StatusCode::NOT_FOUND,
                    Json(json!({ "ok": false, "error": format!("Project '{}' not found and no valid ?root= provided", project_id) })),
                ),
            }
        }
    };

    let adr_dir = PathBuf::from(&root_path).join("docs/adrs");
    if !adr_dir.is_dir() {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({
                "error": "ADR directory not found",
                "searched": [adr_dir.display().to_string()]
            })),
        );
    }

    let adrs = list_adrs_from_dir(&adr_dir);
    (
        StatusCode::OK,
        Json(serde_json::to_value(&adrs).unwrap_or_default()),
    )
}

/// GET /api/projects/{id}/adrs/{adr_id} — get a single ADR from a project.
/// Accepts optional `?root=/abs/path` query param as fallback when project lookup fails.
pub async fn get_project_adr(
    Path((project_id, adr_id)): Path<(String, String)>,
    Query(params): Query<std::collections::HashMap<String, String>>,
    State(state): State<SharedState>,
) -> (StatusCode, Json<serde_json::Value>) {
    let root_path = match resolve_project_path(&state, &project_id).await {
        Ok(p) => p,
        Err(_) => {
            match params.get("root") {
                Some(r) if !r.is_empty() && std::path::Path::new(r).is_dir() => r.clone(),
                _ => return (
                    StatusCode::NOT_FOUND,
                    Json(json!({ "ok": false, "error": format!("Project '{}' not found and no valid ?root= provided", project_id) })),
                ),
            }
        }
    };

    let adr_dir = PathBuf::from(&root_path).join("docs/adrs");
    if !adr_dir.is_dir() {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "ADR directory not found" })),
        );
    }

    match get_adr_from_dir(&adr_dir, &adr_id) {
        Some(detail) => (
            StatusCode::OK,
            Json(serde_json::to_value(&detail).unwrap_or_default()),
        ),
        None => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": format!("ADR-{} not found in project '{}'", adr_id, project_id) })),
        ),
    }
}

/// PUT /api/projects/{id}/adrs/{adr_id} — save ADR content in a project.
pub async fn save_project_adr(
    Path((project_id, adr_id)): Path<(String, String)>,
    State(state): State<SharedState>,
    Json(body): Json<SaveADRRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    let root_path = match resolve_project_path(&state, &project_id).await {
        Ok(p) => p,
        Err((status, body)) => return (status, body),
    };

    let adr_dir = PathBuf::from(&root_path).join("docs/adrs");
    if !adr_dir.is_dir() {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "ADR directory not found" })),
        );
    }

    match save_adr_in_dir(&adr_dir, &adr_id, &body.content) {
        Ok(filename) => (
            StatusCode::OK,
            Json(json!({ "ok": true, "file": filename })),
        ),
        Err(msg) => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": msg })),
        ),
    }
}

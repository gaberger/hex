use axum::{extract::Path, Json};
use http::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::path::PathBuf;

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

/// Find the ADR directory — look in current dir and common project roots.
fn find_adr_dir() -> Option<PathBuf> {
    let candidates = [
        PathBuf::from("docs/adrs"),
        PathBuf::from("../docs/adrs"),
        PathBuf::from("../../docs/adrs"),
    ];
    for c in &candidates {
        if c.is_dir() {
            return Some(c.clone());
        }
    }
    if let Ok(root) = std::env::var("HEX_PROJECT_ROOT") {
        let p = PathBuf::from(root).join("docs/adrs");
        if p.is_dir() {
            return Some(p);
        }
    }
    None
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

/// Extract the numeric ID portion from an ADR filename.
fn extract_id(filename: &str) -> String {
    filename
        .trim_start_matches("ADR-")
        .trim_start_matches("adr-")
        .split('-')
        .next()
        .unwrap_or("000")
        .to_string()
}

/// GET /api/adrs — list all ADRs with metadata.
pub async fn list_adrs() -> (StatusCode, Json<serde_json::Value>) {
    let dir = match find_adr_dir() {
        Some(d) => d,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(json!({
                    "error": "ADR directory not found",
                    "searched": ["docs/adrs", "../docs/adrs", "../../docs/adrs"]
                })),
            )
        }
    };

    let mut adrs: Vec<ADRSummary> = Vec::new();

    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let filename = entry.file_name().to_string_lossy().to_string();
            if !filename.ends_with(".md") {
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

    // Sort by ID descending (newest first)
    adrs.sort_by(|a, b| b.id.cmp(&a.id));

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

    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let filename = entry.file_name().to_string_lossy().to_string();
            if !filename.ends_with(".md") {
                continue;
            }

            let file_id = extract_id(&filename);

            if file_id == id {
                let content = std::fs::read_to_string(entry.path()).unwrap_or_default();
                let (title, status, date) = parse_adr_frontmatter(&content, &filename);

                let detail = ADRDetail {
                    id: id.clone(),
                    title,
                    status,
                    date,
                    content,
                };
                return (
                    StatusCode::OK,
                    Json(serde_json::to_value(&detail).unwrap_or_default()),
                );
            }
        }
    }

    (
        StatusCode::NOT_FOUND,
        Json(json!({ "error": format!("ADR-{} not found", id) })),
    )
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

    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let filename = entry.file_name().to_string_lossy().to_string();
            if !filename.ends_with(".md") {
                continue;
            }

            let file_id = extract_id(&filename);

            if file_id == id {
                return match std::fs::write(entry.path(), &body.content) {
                    Ok(()) => (
                        StatusCode::OK,
                        Json(json!({ "ok": true, "file": filename })),
                    ),
                    Err(e) => (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(json!({ "error": format!("Failed to write ADR: {}", e) })),
                    ),
                };
            }
        }
    }

    (
        StatusCode::NOT_FOUND,
        Json(json!({ "error": format!("ADR-{} not found", id) })),
    )
}

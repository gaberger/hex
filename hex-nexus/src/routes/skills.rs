//! REST endpoints for the skill registry (ADR-042).
//!
//! Reads skills directly from the filesystem — more reliable than SpacetimeDB
//! since module publishing races with config_sync on startup.
//!
//! GET  /api/skills         — list all registered skills
//! GET  /api/skills/{name}  — get a specific skill by name or id
//! POST /api/skills/sync    — re-sync skills from filesystem to SpacetimeDB

use axum::{
    extract::{Path, State},
    Json,
};
use http::StatusCode;
use serde_json::{json, Value};
use std::path::PathBuf;

use crate::state::SharedState;

/// Locate the project root (HEX_PROJECT_ROOT or walk up from cwd).
fn find_project_root() -> Option<PathBuf> {
    if let Ok(root) = std::env::var("HEX_PROJECT_ROOT") {
        let p = PathBuf::from(root);
        if p.is_dir() {
            return Some(p);
        }
    }
    let cwd = std::env::current_dir().ok()?;
    let mut dir = cwd.as_path();
    loop {
        if dir.join("Cargo.toml").exists() || dir.join("package.json").exists() || dir.join(".hex").exists() {
            return Some(dir.to_path_buf());
        }
        dir = dir.parent()?;
    }
}

/// Parse frontmatter from a SKILL.md or *.md skill file.
/// Returns (name, trigger, description).
fn parse_skill_frontmatter(content: &str, fallback_name: &str) -> (String, String, String) {
    let mut name = fallback_name.trim_end_matches(".md").to_string();
    let mut trigger = String::new();
    let mut description = String::new();
    let mut in_frontmatter = false;
    let mut frontmatter_done = false;

    for (i, line) in content.lines().enumerate() {
        if i == 0 && line == "---" {
            in_frontmatter = true;
            continue;
        }
        if in_frontmatter {
            if line == "---" {
                frontmatter_done = true;
                in_frontmatter = false;
                continue;
            }
            if let Some(val) = line.strip_prefix("name:") {
                name = val.trim().trim_matches('"').to_string();
            } else if let Some(val) = line.strip_prefix("trigger:") {
                trigger = val.trim().trim_matches('"').to_string();
            } else if let Some(val) = line.strip_prefix("description:") {
                description = val.trim().trim_matches('"').to_string();
            }
        } else if frontmatter_done && description.is_empty() {
            // Fallback: first non-empty paragraph after frontmatter
            let trimmed = line.trim();
            if !trimmed.is_empty() && !trimmed.starts_with('#') {
                description = trimmed.to_string();
            }
        }
    }

    (name, trigger, description)
}

/// Collect all skills from a skills directory.
/// Handles both subdirectory/SKILL.md and flat *.md patterns.
fn collect_skills_from_dir(dir: &std::path::Path, source_prefix: &str) -> Vec<Value> {
    let mut skills = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else { return skills };

    for entry in entries.flatten() {
        let path = entry.path();
        let filename = entry.file_name().to_string_lossy().to_string();

        if path.is_dir() {
            let skill_md = path.join("SKILL.md");
            if skill_md.exists() {
                if let Ok(content) = std::fs::read_to_string(&skill_md) {
                    let (name, trigger, desc) = parse_skill_frontmatter(&content, &filename);
                    skills.push(json!({
                        "name": name,
                        "trigger": trigger,
                        "description": desc,
                        "source_path": format!("{}/{}/SKILL.md", source_prefix, filename),
                    }));
                }
            }
        } else if filename.ends_with(".md") && !filename.starts_with('.') {
            if let Ok(content) = std::fs::read_to_string(&path) {
                let (name, trigger, desc) = parse_skill_frontmatter(&content, &filename);
                skills.push(json!({
                    "name": name,
                    "trigger": trigger,
                    "description": desc,
                    "source_path": format!("{}/{}", source_prefix, filename),
                }));
            }
        }
    }

    skills
}

/// GET /api/skills — list all registered skills from filesystem.
pub async fn list_skills(
    State(_state): State<SharedState>,
) -> (StatusCode, Json<Value>) {
    let root = match find_project_root() {
        Some(r) => r,
        None => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "Cannot determine project root" })),
            );
        }
    };

    let mut all: Vec<Value> = Vec::new();

    // Global skills: skills/*/SKILL.md
    let global = root.join("skills");
    if global.is_dir() {
        all.extend(collect_skills_from_dir(&global, "skills"));
    }

    // Project skills: .claude/skills/*/SKILL.md and .claude/skills/*.md
    let project = root.join(".claude/skills");
    if project.is_dir() {
        all.extend(collect_skills_from_dir(&project, ".claude/skills"));
    }

    (StatusCode::OK, Json(json!(all)))
}

/// GET /api/skills/{name} — get a specific skill by name.
pub async fn get_skill(
    State(state): State<SharedState>,
    Path(name): Path<String>,
) -> (StatusCode, Json<Value>) {
    // Reuse list_skills to get all, then filter by name
    let (status, Json(all)) = list_skills(State(state)).await;
    if !status.is_success() {
        return (status, Json(all));
    }

    let found = all.as_array()
        .and_then(|arr| arr.iter().find(|s| s["name"].as_str() == Some(&name)).cloned());

    match found {
        Some(skill) => (StatusCode::OK, Json(skill)),
        None => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": format!("Skill '{}' not found", name) })),
        ),
    }
}

/// POST /api/skills/sync — re-sync skills from filesystem → SpacetimeDB.
pub async fn sync_skills(
    State(state): State<SharedState>,
) -> (StatusCode, Json<Value>) {
    let root = match find_project_root() {
        Some(r) => r,
        None => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "Cannot determine project root" })),
            );
        }
    };

    // Count skills from filesystem
    let mut count = 0usize;
    for dir_name in &["skills", ".claude/skills"] {
        let dir = root.join(dir_name);
        if dir.is_dir() {
            count += collect_skills_from_dir(&dir, dir_name).len();
        }
    }

    // Trigger full config sync in background (best-effort, non-blocking)
    let stdb_host = std::env::var("HEX_SPACETIMEDB_HOST")
        .unwrap_or_else(|_| "http://127.0.0.1:3033".to_string());
    let stdb_db = std::env::var("HEX_SPACETIMEDB_DATABASE")
        .unwrap_or_else(|_| "hexflo-coordination".to_string());
    let root_clone = root.clone();
    tokio::spawn(async move {
        crate::config_sync::sync_project_config_with_report(&root_clone, &stdb_host, &stdb_db).await;
    });

    // Also try SpacetimeDB if available
    if let Some(port) = &state.state_port {
        if let Ok(stdb_skills) = port.skill_list().await {
            if !stdb_skills.is_empty() {
                return (StatusCode::OK, Json(json!({ "synced": stdb_skills.len(), "ok": true, "source": "spacetimedb" })));
            }
        }
    }

    (StatusCode::OK, Json(json!({ "synced": count, "ok": true, "source": "filesystem" })))
}

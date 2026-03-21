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

// ---------------------------------------------------------------------------
// Project initialization (scaffolding)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct InitProjectRequest {
    pub path: String,
    pub name: Option<String>,
}

/// POST /api/projects/init — scaffold the standard hex config directory structure.
///
/// Idempotent: only creates files/directories that don't already exist.
pub async fn init_project(
    Json(body): Json<InitProjectRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    let root = PathBuf::from(&body.path);
    if !root.is_dir() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "Path is not a directory" })),
        );
    }

    let name = body.name.unwrap_or_else(|| {
        root.file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string()
    });

    let mut created: Vec<String> = Vec::new();

    // Create .hex/ directory with default config files
    let hex_dir = root.join(".hex");
    if !hex_dir.exists() {
        let _ = std::fs::create_dir_all(&hex_dir);
        created.push(".hex/".to_string());

        // Write default blueprint.json
        let blueprint = json!({
            "layers": [
                { "name": "domain", "path": "src/core/domain/", "imports": [] },
                { "name": "ports", "path": "src/core/ports/", "imports": ["domain"] },
                { "name": "usecases", "path": "src/core/usecases/", "imports": ["domain", "ports"] },
                { "name": "primary", "path": "src/adapters/primary/", "imports": ["ports"] },
                { "name": "secondary", "path": "src/adapters/secondary/", "imports": ["ports"] }
            ],
            "rules": [
                "Adapters must NEVER import other adapters",
                "Domain must only import from domain"
            ]
        });
        let _ = std::fs::write(
            hex_dir.join("blueprint.json"),
            serde_json::to_string_pretty(&blueprint).unwrap_or_default(),
        );
        created.push(".hex/blueprint.json".to_string());

        // Write state.json pointing to SpacetimeDB
        let state = json!({
            "host": "http://127.0.0.1:3000",
            "database": "hexflo-coordination"
        });
        let _ = std::fs::write(
            hex_dir.join("state.json"),
            serde_json::to_string_pretty(&state).unwrap_or_default(),
        );
        created.push(".hex/state.json".to_string());
    }

    // Write default .hex/project.yaml manifest if not present
    let manifest_path = hex_dir.join("project.yaml");
    if !manifest_path.exists() {
        let _ = std::fs::create_dir_all(&hex_dir);
        let manifest = format!(
            "name: {}\ndescription: \"\"\nversion: \"1.0.0\"\nauto_register: true\nagent:\n  provider: auto\n  model: claude-sonnet-4-20250514\n",
            name
        );
        let _ = std::fs::write(&manifest_path, manifest);
        created.push(".hex/project.yaml".to_string());
    }

    // Create .claude/ directories
    let claude_dir = root.join(".claude");
    if !claude_dir.exists() {
        let _ = std::fs::create_dir_all(claude_dir.join("skills"));
        let _ = std::fs::create_dir_all(claude_dir.join("agents"));
        created.push(".claude/skills/".to_string());
        created.push(".claude/agents/".to_string());
    }

    // Copy embedded agent templates into .claude/agents/ (idempotent)
    {
        use crate::templates::{AgentTemplates, SkillTemplates};

        let agents_dir = root.join(".claude/agents");
        let _ = std::fs::create_dir_all(&agents_dir);
        for file_name in AgentTemplates::iter() {
            let file_name_str = file_name.as_ref();
            if let Some(content) = AgentTemplates::get(file_name_str) {
                let dest_name = file_name_str.strip_prefix("agents/").unwrap_or(file_name_str);
                let dest = agents_dir.join(dest_name);
                if !dest.exists() {
                    if let Some(parent) = dest.parent() {
                        let _ = std::fs::create_dir_all(parent);
                    }
                    let _ = std::fs::write(&dest, content.data.as_ref());
                    created.push(format!(".claude/agents/{}", dest_name));
                }
            }
        }

        // Copy embedded skill templates into .claude/skills/ (idempotent)
        let skills_dir = root.join(".claude/skills");
        let _ = std::fs::create_dir_all(&skills_dir);
        for file_name in SkillTemplates::iter() {
            let file_name_str = file_name.as_ref();
            if let Some(content) = SkillTemplates::get(file_name_str) {
                let dest_name = file_name_str.strip_prefix("skills/").unwrap_or(file_name_str);
                let dest = skills_dir.join(dest_name);
                if !dest.exists() {
                    if let Some(parent) = dest.parent() {
                        let _ = std::fs::create_dir_all(parent);
                    }
                    let _ = std::fs::write(&dest, content.data.as_ref());
                    created.push(format!(".claude/skills/{}", dest_name));
                }
            }
        }
    }

    // Create docs/adrs/ with template
    let adrs_dir = root.join("docs").join("adrs");
    if !adrs_dir.exists() {
        let _ = std::fs::create_dir_all(&adrs_dir);
        created.push("docs/adrs/".to_string());

        let readme = format!(
            "# Architecture Decision Records\n\n\
             See the [ADR guide](https://github.com/hex-intf/hex/blob/main/docs/adrs/README.md) for conventions.\n"
        );
        let _ = std::fs::write(adrs_dir.join("README.md"), readme);

        let template = "# ADR-{NNN}: {Title}\n\n\
            **Status:** Proposed\n\
            **Date:** {YYYY-MM-DD}\n\
            **Drivers:** {reason}\n\n\
            ## Context\n\n\
            {description}\n\n\
            ## Decision\n\n\
            {what we decided}\n\n\
            ## Consequences\n\n\
            **Positive:**\n\
            - \n\n\
            **Negative:**\n\
            - \n";
        let _ = std::fs::write(adrs_dir.join("TEMPLATE.md"), template);
        created.push("docs/adrs/TEMPLATE.md".to_string());
    }

    // Create CLAUDE.md if missing
    let claude_md = root.join("CLAUDE.md");
    if !claude_md.exists() {
        let content = format!(
            "# {} -- Project Instructions\n\n\
             ## What This Project Is\n\n\
             {} is a project managed by Hex Nexus.\n\n\
             ## Build & Test\n\n\
             ```bash\n\
             # TODO: Add build commands\n\
             ```\n\n\
             ## Architecture\n\n\
             This project uses hexagonal architecture. See `.hex/blueprint.json` for layer definitions.\n",
            name, name
        );
        let _ = std::fs::write(&claude_md, content);
        created.push("CLAUDE.md".to_string());
    }

    (
        StatusCode::CREATED,
        Json(json!({
            "initialized": true,
            "name": name,
            "path": body.path,
            "created": created,
        })),
    )
}

/// POST /api/config/sync — trigger manual config re-sync from repo files to SpacetimeDB.
pub async fn resync_config() -> (StatusCode, Json<serde_json::Value>) {
    if let Ok(cwd) = std::env::current_dir() {
        let host = std::env::var("HEX_SPACETIMEDB_HOST")
            .unwrap_or_else(|_| "http://127.0.0.1:3000".to_string());
        let db = std::env::var("HEX_SPACETIMEDB_DATABASE")
            .unwrap_or_else(|_| "hexflo-coordination".to_string());
        crate::config_sync::sync_project_config(&cwd, &host, &db).await;
        (StatusCode::OK, Json(json!({ "synced": true })))
    } else {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "Cannot determine working directory" })),
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

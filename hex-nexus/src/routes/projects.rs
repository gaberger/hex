use axum::{
    extract::{Path, State},
    Json,
};
use http::StatusCode;
use serde_json::json;

use crate::ports::state::ProjectRegistration;
use crate::state::{make_project_id, SharedState, WsEnvelope};

pub async fn register(
    State(state): State<SharedState>,
    Json(body): Json<serde_json::Value>,
) -> (StatusCode, Json<serde_json::Value>) {
    let root_path = match body.get("rootPath").and_then(|v| v.as_str()) {
        Some(s) => s.to_string(),
        None => return (StatusCode::BAD_REQUEST, Json(json!({ "error": "Missing rootPath" }))),
    };
    let name_field = body.get("name").and_then(|v| v.as_str()).map(String::from);
    let description = body.get("description").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let ast_is_stub = body.get("astIsStub").and_then(|v| v.as_bool()).unwrap_or(false);

    let id = make_project_id(&root_path);
    let name = name_field
        .unwrap_or_else(|| {
            root_path
                .rsplit('/')
                .next()
                .unwrap_or("unknown")
                .to_string()
        });

    let sp = match state.require_state_port() {
        Ok(sp) => sp.clone(),
        Err(e) => return e,
    };

    // Check if project already exists
    let existing = sp.project_get(&id).await.unwrap_or(None);
    let is_new = existing.is_none();

    if let Err(e) = sp.project_register(ProjectRegistration {
        id: id.clone(),
        name: name.clone(),
        description: description.clone(),
        root_path: root_path.clone(),
        ast_is_stub,
    }).await {
        tracing::error!("Failed to register project: {}", e);
        return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e.to_string() })));
    }

    // Broadcast AFTER successful registration
    if is_new {
        let envelope = WsEnvelope {
            topic: "hub:projects".to_string(),
            event: "project-registered".to_string(),
            data: json!({
                "id": id,
                "name": name,
                "rootPath": root_path,
                "timestamp": chrono::Utc::now().timestamp_millis()
            }),
        };
        if state.ws_tx.send(envelope).is_err() {
            tracing::debug!("WS broadcast: no receivers for project-registered");
        }
    }

    (StatusCode::OK, Json(json!({ "id": id, "name": name, "rootPath": root_path })))
}

pub async fn unregister(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> (StatusCode, Json<serde_json::Value>) {
    let sp = match state.require_state_port() {
        Ok(sp) => sp.clone(),
        Err(e) => return e,
    };

    match sp.project_unregister(&id).await {
        Ok(true) => {
            let envelope = WsEnvelope {
                topic: "hub:projects".to_string(),
                event: "project-unregistered".to_string(),
                data: json!({ "id": id, "timestamp": chrono::Utc::now().timestamp_millis() }),
            };
            if state.ws_tx.send(envelope).is_err() {
                tracing::debug!("WS broadcast: no receivers for project-unregistered");
            }
            (StatusCode::OK, Json(json!({ "ok": true })))
        }
        Ok(false) => (StatusCode::NOT_FOUND, Json(json!({ "error": "Not found" }))),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e.to_string() }))),
    }
}

/// POST /api/projects/:id/archive — unregister + remove .hex/ config, keep source files.
pub async fn archive_project(
    State(state): State<SharedState>,
    Path(id): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> (StatusCode, Json<serde_json::Value>) {
    let sp = match state.require_state_port() {
        Ok(sp) => sp.clone(),
        Err(e) => return e,
    };

    // Get project root path — try state first, fall back to body.path
    let root_path = match sp.project_get(&id).await {
        Ok(Some(p)) => p.root_path,
        _ => match body.get("path").and_then(|v| v.as_str()) {
            Some(p) if !p.is_empty() => p.to_string(),
            _ => return (StatusCode::NOT_FOUND, Json(json!({ "error": "Project not found and no path provided" }))),
        },
    };

    let remove_claude = body.get("removeClaude").and_then(|v| v.as_bool()).unwrap_or(false);
    let root = std::path::Path::new(&root_path);
    let mut removed = Vec::new();

    // Remove .hex/ directory
    let hex_dir = root.join(".hex");
    if hex_dir.exists() {
        let _ = std::fs::remove_dir_all(&hex_dir);
        removed.push(".hex/");
    }

    // Remove .mcp.json
    let mcp_json = root.join(".mcp.json");
    if mcp_json.exists() {
        let _ = std::fs::remove_file(&mcp_json);
        removed.push(".mcp.json");
    }

    // Optionally remove .claude/
    if remove_claude {
        let claude_dir = root.join(".claude");
        if claude_dir.exists() {
            let _ = std::fs::remove_dir_all(&claude_dir);
            removed.push(".claude/");
        }
    }

    // Unregister from state
    let _ = sp.project_unregister(&id).await;

    let envelope = WsEnvelope {
        topic: "hub:projects".to_string(),
        event: "project-archived".to_string(),
        data: json!({ "id": id, "removed": removed }),
    };
    let _ = state.ws_tx.send(envelope);

    (StatusCode::OK, Json(json!({ "ok": true, "removed": removed })))
}

/// POST /api/projects/:id/delete — unregister + delete ALL project files from disk.
pub async fn delete_project(
    State(state): State<SharedState>,
    Path(id): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> (StatusCode, Json<serde_json::Value>) {
    // Require explicit confirmation
    let confirmed = body.get("confirm").and_then(|v| v.as_bool()).unwrap_or(false);
    if !confirmed {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "Confirmation required", "hint": "Send { \"confirm\": true }" })),
        );
    }

    let sp = match state.require_state_port() {
        Ok(sp) => sp.clone(),
        Err(e) => return e,
    };

    // Get project root path — try state first, fall back to body.path
    let root_path = match sp.project_get(&id).await {
        Ok(Some(p)) => p.root_path,
        _ => match body.get("path").and_then(|v| v.as_str()) {
            Some(p) if !p.is_empty() => p.to_string(),
            _ => return (StatusCode::NOT_FOUND, Json(json!({ "error": "Project not found and no path provided" }))),
        },
    };

    let root = std::path::Path::new(&root_path);
    let canon = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let path_str = canon.to_string_lossy();

    // Safety: refuse to delete system directories
    if path_str == "/"
        || path_str.starts_with("/System")
        || path_str.starts_with("/usr")
        || path_str.starts_with("/bin")
        || path_str.starts_with("/sbin")
        || path_str.starts_with("/var")
    {
        return (
            StatusCode::FORBIDDEN,
            Json(json!({ "error": "Refusing to delete protected system path" })),
        );
    }

    // Also refuse home directory
    if let Ok(home) = std::env::var("HOME") {
        if path_str == home {
            return (
                StatusCode::FORBIDDEN,
                Json(json!({ "error": "Refusing to delete home directory" })),
            );
        }
    }

    // Unregister first
    let _ = sp.project_unregister(&id).await;

    // Delete all files
    let deleted = if root.exists() {
        match std::fs::remove_dir_all(root) {
            Ok(()) => true,
            Err(e) => {
                tracing::error!("Failed to delete {}: {}", root_path, e);
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({ "error": format!("Delete failed: {}", e) })),
                );
            }
        }
    } else {
        false
    };

    let envelope = WsEnvelope {
        topic: "hub:projects".to_string(),
        event: "project-deleted".to_string(),
        data: json!({ "id": id, "path": root_path }),
    };
    let _ = state.ws_tx.send(envelope);

    (StatusCode::OK, Json(json!({ "ok": true, "deleted": deleted, "path": root_path })))
}

/// GET /api/projects/:id/report — full project visibility: project → agents → swarms → tasks
pub async fn project_report(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> (StatusCode, Json<serde_json::Value>) {
    let sp = match state.state_port.as_ref() {
        Some(sp) => sp,
        None => return (StatusCode::SERVICE_UNAVAILABLE, Json(json!({ "error": "State port not available" }))),
    };

    // 1. Resolve project — special keywords, then exact ID, name/basename, then prefix match
    let project = {
        // "latest" → most recently registered project (highest registered_at)
        let resolved_id: std::borrow::Cow<str> = if id == "latest" {
            let all = sp.project_list().await.unwrap_or_default();
            match all.into_iter().max_by_key(|p| p.registered_at) {
                Some(p) => std::borrow::Cow::Owned(p.id),
                None => {
                    return (StatusCode::NOT_FOUND, Json(json!({ "error": "No projects registered" })));
                }
            }
        } else {
            std::borrow::Cow::Borrowed(&id)
        };
        let id = resolved_id.as_ref();

        let by_id = sp.project_get(id).await.unwrap_or(None);
        let by_find = if by_id.is_none() { sp.project_find(id).await.unwrap_or(None) } else { None };
        let by_prefix = if by_id.is_none() && by_find.is_none() {
            // Prefix/substring match against name and root_path basename
            let all = sp.project_list().await.unwrap_or_default();
            let q = id.to_lowercase();
            all.into_iter().find(|p| {
                p.name.to_lowercase().starts_with(&q)
                    || p.root_path.rsplit('/').next().unwrap_or("").to_lowercase().starts_with(&q)
                    || p.id.starts_with(&q)
            })
        } else {
            None
        };
        match by_id.or(by_find).or(by_prefix) {
            Some(p) => p,
            None => {
                // Return helpful error listing available projects
                let all = sp.project_list().await.unwrap_or_default();
                let available: Vec<_> = all.iter().map(|p| json!({ "id": p.id, "name": p.name })).collect();
                return (StatusCode::NOT_FOUND, Json(json!({
                    "error": format!("Project '{}' not found", id),
                    "hint": "Use `hex project list` to see registered projects",
                    "available": available,
                })));
            }
        }
    };

    // 2. Live agents for this project
    let all_agents = sp.hex_agent_list().await.unwrap_or_default();
    let live_agents: Vec<_> = all_agents.into_iter()
        .filter(|a| a.get("projectId").and_then(|v| v.as_str()).unwrap_or("") == project.id
                 || a.get("project_id").and_then(|v| v.as_str()).unwrap_or("") == project.id)
        .collect();

    // 3. Swarms for this project (all statuses — active, completed, failed)
    let project_swarms = sp.swarm_list_by_project(&project.id).await.unwrap_or_default();

    // 4. Tasks per swarm + summary counts
    let mut swarm_reports = Vec::new();
    let mut total_tasks = 0usize;
    let mut completed_tasks = 0usize;
    let mut pending_tasks = 0usize;
    let mut failed_tasks = 0usize;
    // Collect unique historical agent IDs from task assignment records
    let mut seen_agent_ids: std::collections::HashSet<String> = live_agents
        .iter()
        .filter_map(|a| a.get("id").or_else(|| a.get("agentId")).and_then(|v| v.as_str()).map(str::to_string))
        .collect();
    let mut historical_agents: Vec<serde_json::Value> = live_agents.clone();

    for swarm in &project_swarms {
        let tasks = sp.swarm_task_list(Some(&swarm.id)).await.unwrap_or_default();
        let t_total = tasks.len();
        let t_completed = tasks.iter().filter(|t| t.status == "completed").count();
        let t_failed = tasks.iter().filter(|t| t.status == "failed").count();
        let t_pending = tasks.iter().filter(|t| t.status == "pending").count();
        let t_in_progress = tasks.iter().filter(|t| t.status == "in_progress").count();

        total_tasks += t_total;
        completed_tasks += t_completed;
        failed_tasks += t_failed;
        pending_tasks += t_pending;

        // Collect agent IDs from task records not already in live agent list
        for task in &tasks {
            if !task.agent_id.is_empty() && seen_agent_ids.insert(task.agent_id.clone()) {
                historical_agents.push(json!({
                    "id": task.agent_id,
                    "agentId": task.agent_id,
                    "name": format!("agent-{}", &task.agent_id[..task.agent_id.len().min(8)]),
                    "status": "offline",
                    "project_id": project.id,
                    "historical": true,
                }));
            }
        }

        swarm_reports.push(json!({
            "id": swarm.id,
            "name": swarm.name,
            "topology": swarm.topology,
            "status": swarm.status,
            "createdAt": swarm.created_at,
            "updatedAt": swarm.updated_at,
            "tasks": {
                "total": t_total,
                "completed": t_completed,
                "pending": t_pending,
                "inProgress": t_in_progress,
                "failed": t_failed,
            },
            "taskList": tasks.iter().map(|t| json!({
                "id": t.id,
                "title": normalize_task_title(&t.title),
                "status": t.status,
                "agentId": t.agent_id,
                "result": t.result,
                "dependsOn": t.depends_on,
                "createdAt": t.created_at,
                "completedAt": t.completed_at,
            })).collect::<Vec<_>>(),
        }));
    }

    let agent_count = historical_agents.len();
    let report = json!({
        "project": {
            "id": project.id,
            "name": project.name,
            "rootPath": project.root_path,
            "registeredAt": project.registered_at,
        },
        "agents": historical_agents,
        "swarms": swarm_reports,
        "summary": {
            "agentCount": agent_count,
            "swarmCount": project_swarms.len(),
            "tasksTotal": total_tasks,
            "tasksCompleted": completed_tasks,
            "tasksPending": pending_tasks,
            "tasksFailed": failed_tasks,
        },
    });

    (StatusCode::OK, Json(report))
}

fn normalize_task_title(title: &str) -> String {
    let trimmed = title.trim();
    if trimmed.starts_with('{') {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(trimmed) {
            if let Some(s) = v.get("description").and_then(|d| d.as_str()) {
                return s.to_string();
            }
        }
    }
    title.to_string()
}

pub async fn list_projects(
    State(state): State<SharedState>,
) -> Json<serde_json::Value> {
    let sp = match state.state_port.as_ref() {
        Some(sp) => sp,
        None => return Json(json!({ "projects": [] })),
    };

    let projects = sp.project_list().await.unwrap_or_default();
    let list: Vec<serde_json::Value> = projects
        .iter()
        .map(|p| {
            json!({
                "id": p.id,
                "name": p.name,
                "rootPath": p.root_path,
                "registeredAt": p.registered_at,
                "lastPushAt": p.last_push_at,
                "astIsStub": p.ast_is_stub,
            })
        })
        .collect();
    Json(json!({ "projects": list }))
}

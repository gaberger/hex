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

        // Build task list with retry labels for duplicate step_ids
        let mut step_counts: std::collections::HashMap<String, u32> = std::collections::HashMap::new();
        let task_list: Vec<serde_json::Value> = tasks.iter().map(|t| {
            let (title, step_id) = normalize_task_title_with_step(&t.title);
            let display_title = if let Some(sid) = step_id {
                let count = step_counts.entry(sid).or_insert(0);
                *count += 1;
                if *count > 1 { format!("{} [retry {}]", title, count) } else { title }
            } else {
                title
            };
            json!({
                "id": t.id,
                "title": display_title,
                "status": t.status,
                "agentId": t.agent_id,
                "result": t.result,
                "dependsOn": t.depends_on,
                "createdAt": t.created_at,
                "completedAt": t.completed_at,
            })
        }).collect();

        // A zombie swarm: all tasks stuck in_progress, none completed or failed.
        // This happens when agents die mid-run. Mark it stale so the CLI skips it.
        // Applies to both "active" and "failed" stored statuses.
        let effective_status = if matches!(swarm.status.as_str(), "active" | "failed")
            && t_total > 0
            && t_in_progress == t_total
            && t_completed == 0
            && t_failed == 0
        {
            "stale"
        } else {
            swarm.status.as_str()
        };

        swarm_reports.push(json!({
            "id": swarm.id,
            "name": swarm.name,
            "topology": swarm.topology,
            "status": effective_status,
            "createdAt": swarm.created_at,
            "updatedAt": swarm.updated_at,
            "tasks": {
                "total": t_total,
                "completed": t_completed,
                "pending": t_pending,
                "inProgress": t_in_progress,
                "failed": t_failed,
            },
            "taskList": task_list,
        }));
    }

    // 5. Scan development artifacts from the filesystem
    let artifacts = scan_project_artifacts(&project.root_path).await;

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
        "artifacts": artifacts,
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

/// Returns `(display_title, Option<step_id>)` for retry-labeling by callers.
fn normalize_task_title_with_step(title: &str) -> (String, Option<String>) {
    let trimmed = title.trim();
    if trimmed.starts_with('{') {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(trimmed) {
            if let Some(s) = v.get("description").and_then(|d| d.as_str()) {
                let desc = s.lines().next().unwrap_or(s).to_string();
                let step_id = v.get("step_id").and_then(|i| i.as_str()).map(str::to_string);
                // If the payload already encodes an iteration, respect it
                if let Some(n) = v.get("iteration").and_then(|i| i.as_u64()) {
                    return (format!("{} [retry {}]", desc, n), step_id);
                }
                return (desc, step_id);
            }
        }
    }
    (title.to_string(), None)
}


/// Scan docs/ directory for ADRs, workplans, and specs.
/// Returns a summary JSON with counts and active items for the report.
async fn scan_project_artifacts(root_path: &str) -> serde_json::Value {
    let root = std::path::Path::new(root_path);

    // ── ADRs ──────────────────────────────────────────────────────────────────
    let mut adr_accepted = 0u32;
    let mut adr_proposed = 0u32;
    let mut adr_deprecated = 0u32;
    let mut adr_list: Vec<serde_json::Value> = Vec::new();

    let adr_dir = root.join("docs").join("adrs");
    if let Ok(mut entries) = tokio::fs::read_dir(&adr_dir).await {
        while let Ok(Some(entry)) = entries.next_entry().await {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("md") {
                continue;
            }
            let fname = path.file_name().and_then(|n| n.to_str()).unwrap_or("").to_string();
            // Extract ADR ID from filename: ADR-XXXXXX-<slug>.md → "ADR-XXXXXX"
            let adr_id = fname.split('-').take(2).collect::<Vec<_>>().join("-");
            // Read first 1 KB to find Status line (avoid reading entire file)
            let mut status = "proposed".to_string();
            if let Ok(content) = tokio::fs::read_to_string(&path).await {
                for line in content.lines().take(30) {
                    let lower = line.to_lowercase();
                    if lower.contains("status:") || lower.contains("## status") {
                        if lower.contains("accepted") { status = "accepted".to_string(); }
                        else if lower.contains("deprecated") || lower.contains("superseded") {
                            status = "deprecated".to_string();
                        } else if lower.contains("proposed") || lower.contains("draft") {
                            status = "proposed".to_string();
                        }
                        break;
                    }
                }
            }
            match status.as_str() {
                "accepted"   => adr_accepted += 1,
                "deprecated" => adr_deprecated += 1,
                _            => adr_proposed += 1,
            }
            // Derive title from slug portion of filename
            let slug = fname
                .trim_end_matches(".md")
                .splitn(3, '-')
                .nth(2)
                .unwrap_or("")
                .replace('-', " ");
            adr_list.push(json!({ "id": adr_id, "slug": slug, "status": status, "file": fname }));
        }
    }
    adr_list.sort_by(|a, b| {
        a["file"].as_str().unwrap_or("").cmp(b["file"].as_str().unwrap_or(""))
    });

    // ── Workplans ─────────────────────────────────────────────────────────────
    let mut workplan_list: Vec<serde_json::Value> = Vec::new();
    let wp_dir = root.join("docs").join("workplans");
    if let Ok(mut entries) = tokio::fs::read_dir(&wp_dir).await {
        while let Ok(Some(entry)) = entries.next_entry().await {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let fname = path.file_name().and_then(|n| n.to_str()).unwrap_or("").to_string();
            if let Ok(content) = tokio::fs::read_to_string(&path).await {
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(&content) {
                    let feature = v["feature"].as_str().unwrap_or(&fname).to_string();
                    // Top-level "status": "done" or "completed" means the workplan is finished,
                    // regardless of individual phase status fields (which may predate this field).
                    let top_done = matches!(
                        v["status"].as_str(),
                        Some("done") | Some("completed") | Some("complete") | Some("superseded")
                    );
                    let phases = v["phases"].as_array();
                    let total = phases.map(|p| p.len()).unwrap_or(0);
                    let done = if top_done {
                        total // treat all phases as done
                    } else {
                        phases.map(|p| {
                            p.iter().filter(|ph| ph["status"].as_str() == Some("done")).count()
                        }).unwrap_or(0)
                    };
                    let active = !top_done && done < total;
                    workplan_list.push(json!({
                        "feature": feature,
                        "file": fname,
                        "totalPhases": total,
                        "donePhases": done,
                        "active": active,
                    }));
                }
            }
        }
    }
    workplan_list.sort_by(|a, b| {
        // Active workplans first, then alphabetical
        let a_active = a["active"].as_bool().unwrap_or(false);
        let b_active = b["active"].as_bool().unwrap_or(false);
        b_active.cmp(&a_active).then_with(|| {
            a["feature"].as_str().unwrap_or("").cmp(b["feature"].as_str().unwrap_or(""))
        })
    });

    // ── Specs ─────────────────────────────────────────────────────────────────
    let mut spec_count = 0u32;
    let spec_dir = root.join("docs").join("specs");
    if let Ok(mut entries) = tokio::fs::read_dir(&spec_dir).await {
        while let Ok(Some(entry)) = entries.next_entry().await {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("json") {
                spec_count += 1;
            }
        }
    }

    json!({
        "adrs": {
            "total": adr_accepted + adr_proposed + adr_deprecated,
            "accepted": adr_accepted,
            "proposed": adr_proposed,
            "deprecated": adr_deprecated,
            "list": adr_list,
        },
        "workplans": {
            "total": workplan_list.len(),
            "active": workplan_list.iter().filter(|w| w["active"].as_bool().unwrap_or(false)).count(),
            "list": workplan_list,
        },
        "specs": {
            "total": spec_count,
        },
    })
}

pub async fn list_projects(
    State(state): State<SharedState>,
) -> Json<serde_json::Value> {
    let sp = match state.state_port.as_ref() {
        Some(sp) => sp,
        None => return Json(json!({ "projects": [] })),
    };

    let all_projects = sp.project_list().await.unwrap_or_default();

    // Deduplicate by root_path — keep the entry with the latest registered_at.
    // Multiple registrations of the same path accumulate when `hex dev` re-registers
    // on every run without checking for an existing record.
    let projects: Vec<_> = {
        let mut by_path: std::collections::HashMap<String, _> = std::collections::HashMap::new();
        for p in all_projects {
            // Normalize key so /PARA/... and /para/... collapse to one entry
            let key = p.root_path.to_lowercase();
            let newer = by_path.get(&key)
                .map(|existing: &crate::ports::state::ProjectRecord| p.registered_at > existing.registered_at)
                .unwrap_or(true);
            if newer {
                by_path.insert(key, p);
            }
        }
        // Sort by registered_at descending so newest appear first
        let mut deduped: Vec<_> = by_path.into_values().collect();
        deduped.sort_by(|a, b| b.registered_at.cmp(&a.registered_at));
        deduped
    };

    // Enrich each project with inferred status — run git checks concurrently.
    let futures: Vec<_> = projects.iter().map(|p| {
        let root = p.root_path.clone();
        async move {
            let path_exists = std::path::Path::new(&root).exists();
            let git_age_days = if path_exists {
                git_commit_age_days(&root).await
            } else {
                None
            };
            infer_project_status(&root, path_exists, git_age_days)
        }
    }).collect();
    let statuses = futures::future::join_all(futures).await;

    let list: Vec<serde_json::Value> = projects
        .iter()
        .zip(statuses.iter())
        .map(|(p, status)| {
            json!({
                "id": p.id,
                "name": p.name,
                "rootPath": p.root_path,
                "registeredAt": p.registered_at,
                "lastPushAt": p.last_push_at,
                "astIsStub": p.ast_is_stub,
                "status": status,
            })
        })
        .collect();
    Json(json!({ "projects": list }))
}

/// Infer a human-readable project status from observable filesystem signals.
fn infer_project_status(root_path: &str, path_exists: bool, git_age_days: Option<i64>) -> &'static str {
    if !path_exists {
        return "orphaned";
    }
    // Scratch: temp dirs, /workspace container mounts, example sub-projects, or home dir
    let home = std::env::var("HOME").unwrap_or_default();
    let is_scratch = root_path.starts_with("/tmp")
        || root_path.starts_with("/private/tmp")
        || root_path == "/workspace"
        || root_path.contains("/examples/")
        || (!home.is_empty() && root_path == home);
    if is_scratch {
        return "scratch";
    }
    match git_age_days {
        None => "untracked",          // no git repo at path
        Some(d) if d <= 7 => "active",
        Some(d) if d <= 30 => "recent",
        Some(_) => "idle",
    }
}

/// Run `git log -1 --format=%ct` at `path` and return age in days.
/// Returns `None` if the directory is not a git repo or git fails.
async fn git_commit_age_days(path: &str) -> Option<i64> {
    let out = tokio::process::Command::new("git")
        .args(["-C", path, "log", "-1", "--format=%ct"])
        .output()
        .await
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let ts_str = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if ts_str.is_empty() {
        return None;
    }
    let ts: i64 = ts_str.parse().ok()?;
    let now = chrono::Utc::now().timestamp();
    Some((now - ts) / 86_400)
}

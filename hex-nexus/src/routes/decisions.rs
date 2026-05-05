//! REST endpoints for decision resolution (ADR-2604131500 P1.4) + the
//! Brain-dashboard decisions aggregator (wp-brain-dashboard M1).
//!
//! GET  /api/decisions                             — aggregated worklist
//! POST /api/{project_id}/decisions/{decision_id}  — legacy WS broadcast
//! POST /api/decisions/{id}                        — resolve via inbox ack

use axum::{
    extract::{Path, Query, State},
    Json,
};
use http::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::state::{DecisionRequest, SharedState, WsEnvelope};

#[derive(Debug, Deserialize, Default)]
pub struct DecisionsQuery {
    /// Optional project ID. When set, scan that project's rootPath/docs/
    /// instead of the nexus daemon's cwd. Use "__global__" to force the
    /// cwd-only behavior even when projects are registered.
    #[serde(default, alias = "projectId")]
    pub project: Option<String>,
}

/// Legacy project-scoped decision handler (WS broadcast).
pub async fn handle_decision(
    State(state): State<SharedState>,
    Path((project_id, decision_id)): Path<(String, String)>,
    Json(body): Json<DecisionRequest>,
) -> (StatusCode, Json<Value>) {
    // Broadcast decision response via WS
    if state.ws_tx.send(WsEnvelope {
        topic: format!("project:{}:decisions", project_id),
        event: "decision-response".to_string(),
        data: json!({
            "decisionId": decision_id,
            "selectedOption": body.selected_option,
            "respondedBy": "human",
            "timestamp": chrono::Utc::now().timestamp_millis()
        }),
    }).is_err() {
        tracing::warn!("WS broadcast failed for decision {}: no receivers", decision_id);
    }

    (StatusCode::OK, Json(json!({ "ok": true })))
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ResolveDecisionRequest {
    pub action: String,
    pub value: Option<String>,
}

/// POST /api/decisions/{id} — resolve a pending decision.
///
/// Acknowledges the corresponding inbox notification and broadcasts the
/// resolution over WebSocket so dashboards update in real time.
pub async fn resolve_decision(
    State(state): State<SharedState>,
    Path(id): Path<u64>,
    Json(body): Json<ResolveDecisionRequest>,
) -> (StatusCode, Json<Value>) {
    // Validate action
    let valid_actions = ["approve", "reject", "override"];
    if !valid_actions.contains(&body.action.as_str()) {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({
                "error": format!(
                    "Invalid action '{}'. Must be one of: {}",
                    body.action,
                    valid_actions.join(", ")
                )
            })),
        );
    }

    let port = match &state.state_port {
        Some(p) => p,
        None => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(json!({ "error": "IStatePort not initialized (no SpacetimeDB backend)" })),
            );
        }
    };

    // Acknowledge the inbox notification (system agent resolves it)
    if let Err(e) = port.inbox_acknowledge(id, "system").await {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": format!("Failed to acknowledge decision: {}", e) })),
        );
    }

    // Broadcast resolution via WS for real-time dashboard updates
    let _ = state.ws_tx.send(WsEnvelope {
        topic: "decisions".to_string(),
        event: "decision-resolved".to_string(),
        data: json!({
            "id": id,
            "action": body.action,
            "value": body.value,
            "resolved_by": "human",
            "resolved_at": chrono::Utc::now().to_rfc3339(),
        }),
    });

    (
        StatusCode::OK,
        Json(json!({
            "ok": true,
            "id": id,
            "action": body.action,
        })),
    )
}

/// GET /api/decisions/preview?id=<decision-id> — return the markdown content
/// of the underlying ADR or workplan task for inline preview in the dashboard.
///
/// Decision id parsed:
///   adr:ADR-XXX-...           → docs/adrs/ADR-XXX-....md
///   blocked:wp-foo:P1.1       → docs/workplans/wp-foo.json (whole file)
///
/// Capped at 64KB so a runaway file doesn't choke the dashboard. Returns
/// `{ kind, path, markdown, truncated, bytes }`.
#[derive(Debug, Deserialize)]
pub struct PreviewQuery {
    pub id: String,
}

pub async fn preview_decision(
    State(_state): State<SharedState>,
    Query(q): Query<PreviewQuery>,
) -> (StatusCode, Json<Value>) {
    let id = q.id.trim();
    if id.is_empty() {
        return (StatusCode::BAD_REQUEST, Json(json!({ "error": "id query param required" })));
    }
    let scan_root = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));

    let (kind, file_path) = if let Some(stripped) = id.strip_prefix("adr:") {
        ("adr", scan_root.join("docs/adrs").join(format!("{}.md", stripped)))
    } else if let Some(stripped) = id.strip_prefix("blocked:") {
        let (wp_id, _task) = stripped.rsplit_once(':')
            .map(|(a, b)| (a.to_string(), b.to_string()))
            .unwrap_or((stripped.to_string(), "".to_string()));
        ("blocked-task", scan_root.join("docs/workplans").join(format!("{}.json", wp_id)))
    } else {
        return (StatusCode::BAD_REQUEST, Json(json!({
            "error": format!("unsupported decision kind for id `{}`", id),
        })));
    };

    let raw = match std::fs::read_to_string(&file_path) {
        Ok(s) => s,
        Err(e) => return (StatusCode::NOT_FOUND, Json(json!({
            "error": format!("preview source not found at {}: {}", file_path.display(), e),
        }))),
    };
    let bytes = raw.len();
    let cap = 64 * 1024;
    let (markdown, truncated) = if raw.len() > cap {
        (format!("{}\n\n*…truncated to {}KB ({}B total)*", &raw[..cap], cap / 1024, bytes), true)
    } else {
        (raw, false)
    };
    (StatusCode::OK, Json(json!({
        "id": id,
        "kind": kind,
        "path": file_path.strip_prefix(&scan_root).unwrap_or(&file_path).display().to_string(),
        "markdown": markdown,
        "truncated": truncated,
        "bytes": bytes,
    })))
}

/// POST /api/decisions/blocked-task/resolve — resolve a blocked workplan task.
///
/// Body: { "id": "blocked:wp-foo:P1.1", "action": "unblock" | "complete" | "abandon", "note": "..." }
///
/// Parses the composite id ("blocked:<workplan_id>:<task_id>"), opens the
/// workplan JSON, locates the task, applies the action:
///   - unblock: clear blocked_reason, set status="pending" (re-queueable)
///   - complete: set status="done" (operator confirms it's actually done)
///   - abandon: set status="abandoned"
/// In all three cases the entry stops appearing in /api/decisions because
/// the aggregator filters on status=="blocked" || blocked_reason.
///
/// Writes via std::fs — no SafeFileWriter here because workplan JSONs are
/// not on the critical-paths list (operator-curated artifacts).
#[derive(Debug, Deserialize, Default)]
pub struct ResolveBlockedTaskRequest {
    pub id: String,
    pub action: String,
    #[serde(default)]
    pub note: Option<String>,
}

pub async fn resolve_blocked_task(
    State(_state): State<SharedState>,
    Json(req): Json<ResolveBlockedTaskRequest>,
) -> (StatusCode, Json<Value>) {
    // Parse "blocked:<workplan_id>:<task_id>" — workplan ids contain '-' and
    // task ids contain '.' but no colons, so split_once on ':' twice is safe.
    let stripped = match req.id.strip_prefix("blocked:") {
        Some(s) => s,
        None => return (StatusCode::BAD_REQUEST, Json(json!({
            "error": format!("expected id starting with 'blocked:', got '{}'", req.id),
        }))),
    };
    let (wp_id, task_id) = match stripped.rsplit_once(':') {
        Some(p) => p,
        None => return (StatusCode::BAD_REQUEST, Json(json!({
            "error": format!("expected 'blocked:<workplan>:<task>' format, got '{}'", req.id),
        }))),
    };
    let valid_actions = ["unblock", "complete", "abandon"];
    if !valid_actions.contains(&req.action.as_str()) {
        return (StatusCode::BAD_REQUEST, Json(json!({
            "error": format!("invalid action '{}'. Must be: {}", req.action, valid_actions.join(", ")),
        })));
    }

    let scan_root = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    let wp_path = scan_root.join("docs/workplans").join(format!("{}.json", wp_id));
    let raw = match std::fs::read_to_string(&wp_path) {
        Ok(r) => r,
        Err(e) => return (StatusCode::NOT_FOUND, Json(json!({
            "error": format!("workplan not found at {}: {}", wp_path.display(), e),
        }))),
    };
    let mut doc: Value = match serde_json::from_str(&raw) {
        Ok(v) => v,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({
            "error": format!("workplan JSON parse failed: {}", e),
        }))),
    };

    let new_status = match req.action.as_str() {
        "unblock" => "pending",
        "complete" => "done",
        _ => "abandoned",
    };

    let mut found = false;
    if let Some(phases) = doc.get_mut("phases").and_then(|v| v.as_array_mut()) {
        for phase in phases {
            if let Some(tasks) = phase.get_mut("tasks").and_then(|v| v.as_array_mut()) {
                for task in tasks {
                    let matches = task.get("id").and_then(|v| v.as_str()) == Some(task_id);
                    if !matches { continue; }
                    found = true;
                    if let Some(obj) = task.as_object_mut() {
                        obj.insert("status".to_string(), json!(new_status));
                        // Clear blocked_reason regardless of action — once
                        // resolved it's no longer blocked.
                        obj.remove("blocked_reason");
                        if let Some(note) = req.note.as_deref() {
                            obj.insert("operator_resolution_note".to_string(), json!(note));
                        }
                        obj.insert("operator_resolved_at".to_string(), json!(chrono::Utc::now().to_rfc3339()));
                    }
                }
            }
        }
    }
    if !found {
        return (StatusCode::NOT_FOUND, Json(json!({
            "error": format!("task '{}' not found in workplan '{}'", task_id, wp_id),
        })));
    }

    let serialized = match serde_json::to_string_pretty(&doc) {
        Ok(s) => s,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({
            "error": format!("workplan re-serialize failed: {}", e),
        }))),
    };
    if let Err(e) = std::fs::write(&wp_path, serialized) {
        return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({
            "error": format!("workplan write failed: {}", e),
        })));
    }
    (StatusCode::OK, Json(json!({
        "ok": true,
        "workplan": wp_id,
        "task": task_id,
        "newStatus": new_status,
    })))
}

/// POST /api/decisions/adr/resolve — resolve a Proposed ADR.
///
/// Body: { "id": "adr:ADR-047-…", "action": "accept" | "reject" | "abandon", "note": "..." }
///
/// Edits the ADR markdown file in place to flip the Status line:
///   - accept   → "**Status:** Accepted"
///   - reject   → "**Status:** Rejected"
///   - abandon  → "**Status:** Abandoned"
/// Once flipped, the ADR no longer matches the "Proposed" filter in the
/// decisions aggregator, so it disappears from the list on next refresh.
#[derive(Debug, Deserialize, Default)]
pub struct ResolveAdrRequest {
    pub id: String,
    pub action: String,
    #[serde(default)]
    pub note: Option<String>,
}

pub async fn resolve_adr(
    State(_state): State<SharedState>,
    Json(req): Json<ResolveAdrRequest>,
) -> (StatusCode, Json<Value>) {
    let stripped = match req.id.strip_prefix("adr:") {
        Some(s) => s,
        None => return (StatusCode::BAD_REQUEST, Json(json!({
            "error": format!("expected id starting with 'adr:', got '{}'", req.id),
        }))),
    };
    let valid = ["accept", "reject", "abandon"];
    if !valid.contains(&req.action.as_str()) {
        return (StatusCode::BAD_REQUEST, Json(json!({
            "error": format!("invalid action '{}'. Must be: {}", req.action, valid.join(", ")),
        })));
    }
    let new_status = match req.action.as_str() {
        "accept" => "Accepted",
        "reject" => "Rejected",
        _ => "Abandoned",
    };

    let scan_root = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    let adr_path = scan_root.join("docs/adrs").join(format!("{}.md", stripped));
    let raw = match std::fs::read_to_string(&adr_path) {
        Ok(r) => r,
        Err(e) => return (StatusCode::NOT_FOUND, Json(json!({
            "error": format!("ADR not found at {}: {}", adr_path.display(), e),
        }))),
    };
    // Replace the first Status: Proposed line. Tolerant of "Status:", "**Status:**",
    // mixed case, and whatever's after Proposed (some have trailing dates).
    let mut found = false;
    let mut updated_lines: Vec<String> = Vec::with_capacity(raw.lines().count());
    for line in raw.lines() {
        if !found {
            let lower = line.to_ascii_lowercase();
            let trimmed = lower.trim();
            if (trimmed.starts_with("**status:**") || trimmed.starts_with("status:"))
                && trimmed.contains("proposed")
            {
                // Preserve original style (bold or plain) by replacing only the word.
                let new_line = if line.trim().to_ascii_lowercase().starts_with("**status:**") {
                    format!("**Status:** {} (resolved {})", new_status, chrono::Utc::now().format("%Y-%m-%d"))
                } else {
                    format!("Status: {} (resolved {})", new_status, chrono::Utc::now().format("%Y-%m-%d"))
                };
                updated_lines.push(new_line);
                found = true;
                continue;
            }
        }
        updated_lines.push(line.to_string());
    }
    if !found {
        return (StatusCode::NOT_FOUND, Json(json!({
            "error": "no 'Status: Proposed' line found in this ADR",
        })));
    }
    let mut new_content = updated_lines.join("\n");
    if !new_content.ends_with('\n') { new_content.push('\n'); }
    if let Some(note) = req.note.as_deref().filter(|s| !s.is_empty()) {
        new_content.push_str(&format!("\n---\n## Operator Resolution Note ({})\n\n{}\n", chrono::Utc::now().to_rfc3339(), note));
    }
    if let Err(e) = std::fs::write(&adr_path, new_content) {
        return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({
            "error": format!("ADR write failed: {}", e),
        })));
    }
    (StatusCode::OK, Json(json!({
        "ok": true,
        "adr": stripped,
        "newStatus": new_status,
    })))
}

// ── Brain-dashboard decisions aggregator (wp-brain-dashboard M1) ──────────

/// One actionable item the operator owes a decision on.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DecisionItem {
    /// Stable identifier — used by the resolve endpoint and the deep-link.
    pub id: String,
    /// Category — drives icon + color in the UI.
    /// One of: blocked_task, proposed_adr, persona_bypass, adversary_disagreement,
    /// reconcile_demotion, stale_workplan.
    pub kind: String,
    /// CRITICAL | HIGH | MEDIUM | LOW.
    pub severity: String,
    /// Short headline, one line.
    pub title: String,
    /// Longer reason / context.
    pub reason: String,
    /// Approximate age in seconds (best-effort; 0 if unknown).
    pub age_seconds: i64,
    /// Suggested operator action — copy intended for a button label.
    pub suggested_action: String,
    /// Deep-link path (relative URL) into the dashboard for context.
    pub link: Option<String>,
    /// In-flight worker dispatch covering this decision (None if no worker
    /// is actively working on it). Operator should NOT re-dispatch when
    /// this is set — duplicates work that's already running.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub in_flight: Option<DecisionInFlight>,
}

/// Lightweight pointer to an inference_task that's covering a decision.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DecisionInFlight {
    pub dispatch_id: String,
    pub role: String,
    pub status: String,
}

/// GET /api/decisions — pull together everything that needs an operator decision.
///
/// Aggregates from multiple sources:
///   - Blocked workplan tasks (status:blocked OR has blocked_reason field)
///   - Proposed ADRs older than 24h
///   - Persona-bypass: dispatch arms with no YAML (Gap B from `hex agent overview`)
///   - Adversary disagreements (red flagged CRITICAL but blue clean, or vice versa)
///   - Reconcile demotions in the last 24h
///
/// The endpoint is best-effort — failures in any single source produce a warn
/// but the rest of the list still returns. Fields that can't be computed
/// (e.g. age when timestamp is missing) default to 0.
pub async fn list_decisions(
    State(state): State<SharedState>,
    Query(q): Query<DecisionsQuery>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    // Resolve scan root. When ?project=<id> is set and resolves to a
    // ProjectRecord, walk THAT project's docs tree. Otherwise fall back to
    // the nexus daemon's cwd (legacy behavior).
    let project_id_filter = q.project.as_deref().unwrap_or("");
    let scan_root: std::path::PathBuf = if !project_id_filter.is_empty() && project_id_filter != "__global__" {
        if let Some(port) = state.state_port.as_ref() {
            match port.project_get(project_id_filter).await {
                Ok(Some(p)) => std::path::PathBuf::from(p.root_path),
                _ => std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from(".")),
            }
        } else {
            std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."))
        }
    } else {
        std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."))
    };
    let workplans_dir = scan_root.join("docs/workplans");
    let adrs_dir = scan_root.join("docs/adrs");

    let mut items: Vec<DecisionItem> = Vec::new();

    // Source 1: Blocked workplan tasks (read from filesystem because workplans
    // live in docs/workplans/wp-*.json, not just STDB). Best-effort — we look
    // in the project working directory if available.
    if let Ok(entries) = std::fs::read_dir(&workplans_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("json") {
                continue;
            }
            let Some(name) = path.file_stem().and_then(|s| s.to_str()) else { continue };
            if !name.starts_with("wp-") {
                continue;
            }
            let Ok(content) = std::fs::read_to_string(&path) else { continue };
            let Ok(root): Result<Value, _> = serde_json::from_str(&content) else { continue };
            // Skip paused workplans — operator already deferred them.
            if root.get("paused").and_then(|v| v.as_bool()).unwrap_or(false) {
                continue;
            }
            let workplan_id = root.get("id").and_then(|v| v.as_str()).unwrap_or(name);
            let Some(phases) = root.get("phases").and_then(|v| v.as_array()) else { continue };
            for phase in phases {
                let Some(tasks) = phase.get("tasks").and_then(|v| v.as_array()) else { continue };
                for task in tasks {
                    let task_id = task.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
                    let status = task.get("status").and_then(|v| v.as_str()).unwrap_or("");
                    let blocked_reason = task.get("blocked_reason").and_then(|v| v.as_str()).map(|s| s.to_string());
                    if status == "blocked" || blocked_reason.is_some() {
                        let title = task.get("title").and_then(|v| v.as_str()).unwrap_or("").to_string();
                        items.push(DecisionItem {
                            id: format!("blocked:{}:{}", workplan_id, task_id),
                            kind: "blocked_task".to_string(),
                            severity: "HIGH".to_string(),
                            title: format!("{} {}: {}", workplan_id, task_id, title),
                            reason: blocked_reason.unwrap_or_else(|| "Task marked blocked without reason field".to_string()),
                            age_seconds: 0,
                            suggested_action: "Decide and unblock".to_string(),
                            link: Some(format!("/workplan/{}", workplan_id)),
                            in_flight: None,
                        });
                    }
                }
            }
        }
    }

    // Source 2: Proposed ADRs (older than 24h). Best-effort filesystem scan.
    if let Ok(entries) = std::fs::read_dir(&adrs_dir) {
        let now = chrono::Utc::now().timestamp();
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("md") {
                continue;
            }
            let Some(name) = path.file_stem().and_then(|s| s.to_str()) else { continue };
            let Ok(content) = std::fs::read_to_string(&path) else { continue };
            // Match the canonical "**Status:** Proposed" line we use in templates.
            let is_proposed = content.lines().any(|l| {
                let trimmed = l.trim();
                let lower = trimmed.to_lowercase();
                (lower.starts_with("**status:**") || lower.starts_with("status:"))
                    && lower.contains("proposed")
            });
            if !is_proposed {
                continue;
            }
            let age_seconds = std::fs::metadata(&path)
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| now - d.as_secs() as i64)
                .unwrap_or(0);
            // Only surface ADRs older than 24h.
            if age_seconds < 24 * 3600 {
                continue;
            }
            items.push(DecisionItem {
                id: format!("adr:{}", name),
                kind: "proposed_adr".to_string(),
                severity: "MEDIUM".to_string(),
                title: format!("ADR aging in Proposed: {}", name),
                reason: "ADR has been Status: Proposed for more than 24h. Either move to Accepted, supersede, or close.".to_string(),
                age_seconds,
                suggested_action: "Review + accept or close".to_string(),
                link: Some(format!("/adr/{}", name)),
                in_flight: None,
            });
        }
    }

    // Source 3: Persona bypass — dispatch arms exist with no matching YAML.
    // Today the only known offenders historically were hex-documenter + hex-ux
    // before they got YAMLs. We check by listing assets/agents/hex/hex/ and
    // comparing to the hardcoded list of specialized arms (kept in sync with
    // agent/mod.rs:1830 match arms).
    let specialized_roles: &[&str] = &[
        "hex-coder", "hex-tester", "hex-reviewer",
        "hex-documenter", "hex-ux", "hex-fixer",
    ];
    // Look in both project-local and user-level YAML registries.
    let yaml_dirs = [
        std::path::PathBuf::from(".claude/agents/hex/hex"),
        dirs::home_dir()
            .map(|h| h.join(".claude/agents/hex/hex"))
            .unwrap_or_default(),
    ];
    let mut available: std::collections::HashSet<String> = std::collections::HashSet::new();
    for dir in &yaml_dirs {
        if let Ok(entries) = std::fs::read_dir(dir) {
            for e in entries.flatten() {
                let p = e.path();
                if p.extension().and_then(|s| s.to_str()) == Some("yml") {
                    if let Some(stem) = p.file_stem().and_then(|s| s.to_str()) {
                        available.insert(stem.to_string());
                    }
                }
            }
        }
    }
    for role in specialized_roles {
        if !available.contains(*role) {
            items.push(DecisionItem {
                id: format!("persona-bypass:{}", role),
                kind: "persona_bypass".to_string(),
                severity: "CRITICAL".to_string(),
                title: format!("Persona bypass: {} dispatch arm has no YAML", role),
                reason: "This worker dispatch arm executes without a YAML persona contract — no model.tier, no constraints, no workflow. Backfill the YAML or remove the arm.".to_string(),
                age_seconds: 0,
                suggested_action: "Backfill YAML or remove arm".to_string(),
                link: Some(format!("/persona/{}", role)),
                in_flight: None,
            });
        }
    }

    // Source 4: Inbox unacked priority-2 notifications (ADR-060 — these
    // override current work, so they're definitionally decisions the operator
    // owes attention to right now).
    if let Some(port) = state.state_port.as_ref() {
        // unacked_only=true → only entries that still need acknowledgement.
        if let Ok(rows) = port.inbox_query("system", Some(2), true).await {
            for row in &rows {
                items.push(DecisionItem {
                    id: format!("inbox:{}", row.id),
                    kind: "priority_inbox".to_string(),
                    severity: "CRITICAL".to_string(),
                    title: row.payload.lines().next().unwrap_or("(no message)").to_string(),
                    reason: row.payload.clone(),
                    age_seconds: 0,
                    suggested_action: "Acknowledge".to_string(),
                    link: Some("/inbox".to_string()),
                    in_flight: None,
                });
            }
        }
    }

    // Cross-reference each decision against in-flight inference_tasks.
    // Tags items whose subject is currently being worked on by a worker
    // dispatch so the operator doesn't accidentally re-dispatch (which
    // duplicates work + burns tokens). Best-effort: failures degrade to
    // no tagging.
    if let Some(port) = state.state_port.as_ref() {
        if let Ok(rows) = port.inference_task_list_all().await {
            let in_flight_tasks: Vec<_> = rows
                .iter()
                .filter(|t| matches!(t.status.as_str(), "Pending" | "PendingReview" | "InProgress"))
                .collect();
            for it in items.iter_mut() {
                // Build the search keys from the decision id.
                // blocked:wp-foo:P1.1 → match prompts containing "wp-foo" + "P1.1"
                // adr:ADR-047-... → match prompts containing "ADR-047"
                let needles: Vec<String> = if it.id.starts_with("blocked:") {
                    let stripped = it.id.strip_prefix("blocked:").unwrap_or("");
                    if let Some((wp, task)) = stripped.rsplit_once(':') {
                        vec![wp.to_string(), task.to_string()]
                    } else { continue; }
                } else if it.id.starts_with("adr:") {
                    let stripped = it.id.strip_prefix("adr:").unwrap_or("");
                    // Match against the leading "ADR-XXX" segment, not the full slug,
                    // so dispatches that say "ADR-047" match "adr:ADR-047-internal-...".
                    let key = stripped.split('-').take(2).collect::<Vec<_>>().join("-");
                    vec![key]
                } else {
                    continue;
                };
                let lneedles: Vec<String> = needles.iter().map(|n| n.to_lowercase()).collect();
                let matched = in_flight_tasks.iter().find(|t| {
                    let prompt_lo = t.prompt.to_lowercase();
                    lneedles.iter().all(|n| prompt_lo.contains(n))
                });
                if let Some(t) = matched {
                    it.in_flight = Some(DecisionInFlight {
                        dispatch_id: t.id.clone(),
                        role: t.role.clone(),
                        status: t.status.clone(),
                    });
                }
            }
        }
    }

    // Sort by severity (CRITICAL first), then by age desc.
    let severity_rank = |s: &str| match s {
        "CRITICAL" => 0,
        "HIGH" => 1,
        "MEDIUM" => 2,
        "LOW" => 3,
        _ => 4,
    };
    items.sort_by(|a, b| {
        severity_rank(&a.severity)
            .cmp(&severity_rank(&b.severity))
            .then(b.age_seconds.cmp(&a.age_seconds))
    });

    let counts_by_severity = {
        let mut m = std::collections::HashMap::<String, u64>::new();
        for it in &items {
            *m.entry(it.severity.clone()).or_default() += 1;
        }
        m
    };

    // camelCase for frontend consumption — matches DecisionItem's serde
    // rename_all attribute. The json! macro doesn't auto-rename so spell
    // it explicitly here.
    Ok(Json(json!({
        "items": items,
        "total": items.len(),
        "bySeverity": counts_by_severity,
        "generatedAt": chrono::Utc::now().to_rfc3339(),
    })))
}

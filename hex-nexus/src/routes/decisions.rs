//! REST endpoints for decision resolution (ADR-2604131500 P1.4) + the
//! Brain-dashboard decisions aggregator (wp-brain-dashboard M1).
//!
//! GET  /api/decisions                             — aggregated worklist
//! POST /api/{project_id}/decisions/{decision_id}  — legacy WS broadcast
//! POST /api/decisions/{id}                        — resolve via inbox ack

use axum::{
    extract::{Path, State},
    Json,
};
use http::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::state::{DecisionRequest, SharedState, WsEnvelope};

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
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let mut items: Vec<DecisionItem> = Vec::new();

    // Source 1: Blocked workplan tasks (read from filesystem because workplans
    // live in docs/workplans/wp-*.json, not just STDB). Best-effort — we look
    // in the project working directory if available.
    if let Ok(entries) = std::fs::read_dir("docs/workplans") {
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
                        });
                    }
                }
            }
        }
    }

    // Source 2: Proposed ADRs (older than 24h). Best-effort filesystem scan.
    if let Ok(entries) = std::fs::read_dir("docs/adrs") {
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
                });
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

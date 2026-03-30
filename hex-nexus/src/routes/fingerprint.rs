//! Architecture fingerprint routes — ADR-2603301200.
//!
//! POST /api/projects/{id}/fingerprint  — generate + store fingerprint
//! GET  /api/projects/{id}/fingerprint  — retrieve as JSON
//! GET  /api/projects/{id}/fingerprint/text — retrieve as formatted injection block

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use serde_json::{json, Value};
use std::path::PathBuf;
use tracing::{info, warn};

use crate::analysis::fingerprint_extractor::FingerprintExtractor;
use crate::state::SharedState;

// ── POST /api/projects/{id}/fingerprint ──────────────────────────────────────

pub async fn generate_fingerprint(
    Path(project_id): Path<String>,
    State(state): State<SharedState>,
    Json(body): Json<Value>,
) -> (StatusCode, Json<Value>) {
    let project_root = match body["project_root"].as_str() {
        Some(r) => PathBuf::from(r),
        None => {
            // Fall back to looking up the project in state
            let sp = match state.state_port.as_ref() {
                Some(sp) => sp,
                None => {
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(json!({ "error": "project_root is required" })),
                    );
                }
            };
            match sp.project_get(&project_id).await {
                Ok(Some(p)) => PathBuf::from(&p.root_path),
                _ => {
                    return (
                        StatusCode::NOT_FOUND,
                        Json(json!({ "error": format!("Project '{}' not found", project_id) })),
                    );
                }
            }
        }
    };

    if !project_root.is_dir() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": format!("project_root '{}' does not exist", project_root.display()) })),
        );
    }

    let workplan_path = body["workplan_path"]
        .as_str()
        .map(PathBuf::from);

    let fp = FingerprintExtractor::extract(
        &project_id,
        &project_root,
        workplan_path.as_deref(),
    ).await;

    info!(
        project_id = %project_id,
        language = %fp.language,
        framework = %fp.framework,
        output_type = %fp.output_type,
        tokens = fp.fingerprint_tokens,
        "architecture fingerprint generated"
    );

    let fp_json = match serde_json::to_value(&fp) {
        Ok(v) => v,
        Err(e) => {
            warn!(error = %e, "failed to serialize fingerprint");
            return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e.to_string() })));
        }
    };

    state.fingerprints.write().await.insert(project_id.clone(), fp);

    (StatusCode::OK, Json(fp_json))
}

// ── GET /api/projects/{id}/fingerprint ───────────────────────────────────────

pub async fn get_fingerprint(
    Path(project_id): Path<String>,
    State(state): State<SharedState>,
) -> (StatusCode, Json<Value>) {
    match state.fingerprints.read().await.get(&project_id) {
        Some(fp) => match serde_json::to_value(fp) {
            Ok(v) => (StatusCode::OK, Json(v)),
            Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e.to_string() }))),
        },
        None => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": format!("No fingerprint for project '{}' — run hex dev to generate", project_id) })),
        ),
    }
}

// ── GET /api/projects/{id}/fingerprint/text ───────────────────────────────────

pub async fn get_fingerprint_text(
    Path(project_id): Path<String>,
    State(state): State<SharedState>,
) -> (StatusCode, String) {
    match state.fingerprints.read().await.get(&project_id) {
        Some(fp) => (StatusCode::OK, fp.to_injection_block()),
        None => (
            StatusCode::NOT_FOUND,
            format!("No fingerprint for project '{}' — run hex dev to generate", project_id),
        ),
    }
}

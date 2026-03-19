//! Architecture analysis routes (ADR-034 Phase 4).
//!
//! Provides on-demand analysis endpoints that use the native tree-sitter
//! adapter — no dependency on the TypeScript CLI.
//!
//! Routes:
//! - `POST /api/analyze` — analyze a project by root path (JSON body)
//! - `GET  /api/{project_id}/analyze` — analyze a registered project (human-readable)
//! - `GET  /api/{project_id}/analyze/json` — analyze a registered project (structured JSON)

use axum::{
    extract::{Path, State},
    Json,
};
use http::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::Arc;

use crate::analysis::analyzer::ArchAnalyzer;
use crate::analysis::ports::ArchAnalysisPort;
use crate::analysis::treesitter_adapter::TreeSitterAdapter;
use crate::state::SharedState;

// ── Request / Response Types ─────────────────────────────

#[derive(Debug, Deserialize)]
pub struct AnalyzeRequest {
    /// Absolute path to the project root to analyze.
    pub root_path: String,
}

#[derive(Debug, Serialize)]
pub struct AnalyzeResponse {
    pub health_score: u8,
    pub file_count: usize,
    pub edge_count: usize,
    pub violation_count: usize,
    pub dead_export_count: usize,
    pub circular_dep_count: usize,
    pub orphan_file_count: usize,
    pub unused_port_count: usize,
    pub violations: serde_json::Value,
    pub dead_exports: serde_json::Value,
    pub circular_deps: Vec<Vec<String>>,
    pub orphan_files: Vec<String>,
    pub unused_ports: Vec<String>,
}

// ── Handlers ─────────────────────────────────────────────

/// POST /api/analyze — analyze any directory by path.
pub async fn analyze_path(
    Json(body): Json<AnalyzeRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    let root = std::path::Path::new(&body.root_path);
    if !root.is_dir() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": format!("'{}' is not a directory", body.root_path) })),
        );
    }

    let analyzer = make_analyzer();
    match analyzer.analyze(root).await {
        Ok(result) => (StatusCode::OK, Json(serde_json::to_value(&result).unwrap_or_default())),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e.to_string() })),
        ),
    }
}

/// GET /api/{project_id}/analyze — analyze a registered project (structured JSON).
pub async fn analyze_project(
    State(state): State<SharedState>,
    Path(project_id): Path<String>,
) -> (StatusCode, Json<serde_json::Value>) {
    let root_path = {
        let projects = state.projects.read().await;
        match projects.get(&project_id) {
            Some(entry) => entry.root_path.clone(),
            None => {
                return (
                    StatusCode::NOT_FOUND,
                    Json(json!({ "error": format!("Project '{}' not found", project_id) })),
                )
            }
        }
    };

    let root = std::path::Path::new(&root_path);
    if !root.is_dir() {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": format!("Project root '{}' is not a directory", root_path) })),
        );
    }

    let analyzer = make_analyzer();
    match analyzer.analyze(root).await {
        Ok(result) => {
            let response = AnalyzeResponse {
                health_score: result.health_score,
                file_count: result.file_count,
                edge_count: result.edge_count,
                violation_count: result.violations.len(),
                dead_export_count: result.dead_exports.len(),
                circular_dep_count: result.circular_deps.len(),
                orphan_file_count: result.orphan_files.len(),
                unused_port_count: result.unused_ports.len(),
                violations: serde_json::to_value(&result.violations).unwrap_or_default(),
                dead_exports: serde_json::to_value(&result.dead_exports).unwrap_or_default(),
                circular_deps: result.circular_deps,
                orphan_files: result.orphan_files,
                unused_ports: result.unused_ports,
            };
            (StatusCode::OK, Json(serde_json::to_value(&response).unwrap_or_default()))
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e.to_string() })),
        ),
    }
}

/// GET /api/{project_id}/analyze/text — human-readable analysis report.
pub async fn analyze_project_text(
    State(state): State<SharedState>,
    Path(project_id): Path<String>,
) -> (StatusCode, String) {
    let root_path = {
        let projects = state.projects.read().await;
        match projects.get(&project_id) {
            Some(entry) => entry.root_path.clone(),
            None => return (StatusCode::NOT_FOUND, format!("Project '{}' not found", project_id)),
        }
    };

    let root = std::path::Path::new(&root_path);
    if !root.is_dir() {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Project root '{}' is not a directory", root_path),
        );
    }

    let analyzer = make_analyzer();
    match analyzer.analyze(root).await {
        Ok(result) => {
            let mut report = String::new();
            report.push_str(&format!("=== Architecture Analysis: {} ===\n\n", project_id));
            report.push_str(&format!("Health Score: {}/100\n", result.health_score));
            report.push_str(&format!("Files: {}  Edges: {}\n\n", result.file_count, result.edge_count));

            if result.violations.is_empty() {
                report.push_str("Boundary Violations: NONE (clean)\n");
            } else {
                report.push_str(&format!("Boundary Violations: {}\n", result.violations.len()));
                for v in &result.violations {
                    report.push_str(&format!(
                        "  {} -> {} : {}\n",
                        v.edge.from_file, v.edge.to_file, v.rule
                    ));
                }
            }

            if result.circular_deps.is_empty() {
                report.push_str("\nCircular Dependencies: NONE\n");
            } else {
                report.push_str(&format!("\nCircular Dependencies: {}\n", result.circular_deps.len()));
                for cycle in &result.circular_deps {
                    report.push_str(&format!("  {}\n", cycle.join(" -> ")));
                }
            }

            if !result.dead_exports.is_empty() {
                report.push_str(&format!("\nDead Exports: {}\n", result.dead_exports.len()));
                for d in &result.dead_exports {
                    report.push_str(&format!("  {}:{} — {}\n", d.file, d.line, d.export_name));
                }
            }

            if !result.orphan_files.is_empty() {
                report.push_str(&format!("\nOrphan Files: {}\n", result.orphan_files.len()));
                for f in &result.orphan_files {
                    report.push_str(&format!("  {}\n", f));
                }
            }

            if !result.unused_ports.is_empty() {
                report.push_str(&format!("\nUnused Ports: {}\n", result.unused_ports.len()));
                for p in &result.unused_ports {
                    report.push_str(&format!("  {}\n", p));
                }
            }

            (StatusCode::OK, report)
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Analysis failed: {}", e)),
    }
}

// ── Helpers ──────────────────────────────────────────────

fn make_analyzer() -> ArchAnalyzer {
    let ast = Arc::new(TreeSitterAdapter::new());
    ArchAnalyzer::new(ast)
}

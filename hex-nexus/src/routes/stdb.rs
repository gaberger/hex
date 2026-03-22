//! SpacetimeDB hydration endpoints.
//!
//! POST /api/stdb/hydrate — publish all WASM modules in tiered dependency order,
//!   coordinated through HexFlo so the dashboard shows real-time progress.
//! GET  /api/stdb/health  — report per-module publish status and overall hydration state.

use axum::extract::State;
use axum::Json;
use http::StatusCode;
use serde::Deserialize;
use serde_json::json;
use std::path::PathBuf;

use crate::spacetime_launcher::{MODULE_TIERS, ModulePublishStatus};
use crate::state::SharedState;

#[derive(Debug, Deserialize)]
pub struct HydrateRequest {
    pub host: Option<String>,
    pub database: Option<String>,
    pub force: Option<bool>,
    pub dry_run: Option<bool>,
}

/// POST /api/stdb/hydrate
///
/// Publishes all WASM modules in tiered dependency order using
/// [`spacetime_launcher::publish_modules_ordered`].
/// Creates a HexFlo swarm to track progress in real-time on the dashboard.
pub async fn hydrate(
    State(state): State<SharedState>,
    Json(body): Json<HydrateRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    let host = body.host.unwrap_or_else(|| "http://127.0.0.1:3000".to_string());
    let database = body.database.unwrap_or_else(|| "hex".to_string());
    let force = body.force.unwrap_or(false);
    let dry_run = body.dry_run.unwrap_or(false);

    // Dry run — return the plan without executing
    if dry_run {
        let tiers: Vec<serde_json::Value> = MODULE_TIERS
            .iter()
            .enumerate()
            .map(|(i, modules)| {
                json!({
                    "tier": i,
                    "modules": modules,
                    "total": modules.len(),
                })
            })
            .collect();

        return (
            StatusCode::OK,
            Json(json!({
                "status": "dry_run",
                "total_modules": MODULE_TIERS.iter().map(|t| t.len()).sum::<usize>(),
                "tiers": tiers,
            })),
        );
    }

    // Verify SpacetimeDB is reachable
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(3))
        .build()
        .unwrap();

    let ping_ok = client
        .get(format!("{}/v1/ping", host))
        .send()
        .await
        .map(|r| r.status().is_success())
        .unwrap_or(false);

    if !ping_ok {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({
                "error": format!("SpacetimeDB not reachable at {}", host),
                "hint": "Start with: hex stdb start",
            })),
        );
    }

    // Create a HexFlo swarm to track this hydration (optional — continues without it)
    let hexflo = state.hexflo.as_ref();

    let swarm_id = if let Some(hf) = hexflo {
        match hf.swarm_init("hex-hydrate", "stdb-hydrate", Some("pipeline".to_string()), Some("hex-nexus")).await {
            Ok(s) => s.id,
            Err(_) => String::new(),
        }
    } else {
        String::new()
    };

    // Discover module source directories
    let modules_base = find_modules_dir();
    let modules_base = match modules_base {
        Some(p) => p,
        None => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({
                    "error": "spacetime-modules/ directory not found",
                    "hint": "Run hex nexus from the project root",
                })),
            );
        }
    };

    // Delegate to the ordered publisher
    let result = match crate::spacetime_launcher::publish_modules_ordered(
        &host,
        &database,
        &modules_base,
        force,
    )
    .await
    {
        Ok(r) => r,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({
                    "error": format!("Hydration failed: {}", e),
                })),
            );
        }
    };

    // Report per-tier results to HexFlo
    for tier in &result.tiers {
        if !swarm_id.is_empty() {
            if let Some(hf) = hexflo {
                let tier_task_title = format!("hydrate-tier-{}", tier.tier);
                if let Ok(task) = hf.task_create(&swarm_id, &tier_task_title).await {
                    let summary = format!(
                        "tier-{}: {}/{} modules published",
                        tier.tier, tier.ok, tier.total
                    );
                    let _ = hf.task_complete(&task.id, Some(summary), None).await;
                }
            }
        }
    }

    // Run config sync with reporting (T7)
    let config_report = if let Ok(cwd) = std::env::current_dir() {
        let stdb_db = std::env::var("HEX_SPACETIMEDB_DATABASE")
            .unwrap_or_else(|_| "hexflo-coordination".to_string());
        Some(crate::config_sync::sync_project_config_with_report(&cwd, &host, &stdb_db).await)
    } else {
        None
    };

    // Teardown the HexFlo swarm
    if let (Some(hf), false) = (hexflo, swarm_id.is_empty()) {
        let _ = hf.swarm_teardown(&swarm_id).await;
    }

    let tiers_json: Vec<serde_json::Value> = result
        .tiers
        .iter()
        .map(|t| {
            let modules: Vec<serde_json::Value> = t
                .modules
                .iter()
                .map(|m| {
                    let status_str = match m.status {
                        ModulePublishStatus::Ok => "ok",
                        ModulePublishStatus::Skipped => "skipped",
                        ModulePublishStatus::Failed => "failed",
                    };
                    json!({
                        "name": m.name,
                        "status": status_str,
                        "error": m.error,
                    })
                })
                .collect();
            json!({
                "tier": t.tier,
                "ok": t.ok,
                "total": t.total,
                "modules": modules,
            })
        })
        .collect();

    (
        StatusCode::OK,
        Json(json!({
            "status": result.status(),
            "modules_published": result.total_ok,
            "modules_failed": result.total_failed,
            "modules_skipped": result.total_skipped,
            "schema_verified": result.schema_verified,
            "tiers": tiers_json,
            "config_sync": config_report,
            "swarm_id": swarm_id,
            "host": host,
            "database": database,
        })),
    )
}

/// GET /api/stdb/health
///
/// Returns the hydration state of SpacetimeDB: connection, per-module status, config sync.
pub async fn health(
    State(state): State<SharedState>,
) -> (StatusCode, Json<serde_json::Value>) {
    let host = std::env::var("HEX_SPACETIMEDB_HOST")
        .unwrap_or_else(|_| "http://127.0.0.1:3000".to_string());
    let database = std::env::var("HEX_SPACETIMEDB_DATABASE")
        .unwrap_or_else(|_| "hex".to_string());

    // Check SpacetimeDB connectivity
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(2))
        .build()
        .unwrap();

    let stdb_reachable = client
        .get(format!("{}/v1/ping", host))
        .send()
        .await
        .map(|r| r.status().is_success())
        .unwrap_or(false);

    // Check if we have an active state connection
    let state_connected = match &state.hexflo {
        Some(hf) => hf.swarm_status().await.is_ok(),
        None => false,
    };

    let overall_status = if !stdb_reachable {
        "disconnected"
    } else if state_connected {
        "hydrated"
    } else {
        "connected_no_state"
    };

    (
        StatusCode::OK,
        Json(json!({
            "status": overall_status,
            "spacetimedb": {
                "host": host,
                "database": database,
                "reachable": stdb_reachable,
            },
            "state_connected": state_connected,
            "module_tiers": MODULE_TIERS.len(),
            "total_modules": MODULE_TIERS.iter().map(|t| t.len()).sum::<usize>(),
        })),
    )
}

// ── Helpers ──────────────────────────────────────────────

fn find_modules_dir() -> Option<PathBuf> {
    // Try CWD first
    let cwd = std::env::current_dir().ok()?;
    let candidate = cwd.join("spacetime-modules");
    if candidate.is_dir() {
        return Some(candidate);
    }

    // Try relative to the binary (for installed hex-nexus)
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            let candidate = parent.join("../../spacetime-modules");
            if candidate.is_dir() {
                return Some(candidate.canonicalize().ok()?);
            }
        }
    }

    // Try HEX_PROJECT_ROOT env var
    if let Ok(root) = std::env::var("HEX_PROJECT_ROOT") {
        let candidate = PathBuf::from(root).join("spacetime-modules");
        if candidate.is_dir() {
            return Some(candidate);
        }
    }

    None
}

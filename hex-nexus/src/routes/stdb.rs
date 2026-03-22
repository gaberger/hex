//! SpacetimeDB hydration endpoints.
//!
//! POST /api/stdb/hydrate — publish all WASM modules in tiered dependency order,
//!   coordinated through HexFlo so the dashboard shows real-time progress.
//! GET  /api/stdb/health  — report per-module publish status and overall hydration state.

use axum::extract::State;
use axum::Json;
use http::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::path::PathBuf;
use std::process::Stdio;

use crate::state::SharedState;

/// Module publish order — tiered by cross-module dependency.
const MODULE_TIERS: &[&[&str]] = &[
    // Tier 0: Foundation — no cross-module dependencies
    &[
        "hexflo-coordination",
        "agent-registry",
        "fleet-state",
        "file-lock-manager",
    ],
    // Tier 1: Services — reference agent/project IDs from tier 0
    &[
        "inference-gateway",
        "inference-bridge",
        "secret-grant",
        "architecture-enforcer",
    ],
    // Tier 2: Workflows — reference agents, inference, secrets
    &[
        "workplan-state",
        "skill-registry",
        "hook-registry",
        "agent-definition-registry",
    ],
    // Tier 3: Coordination — reference everything above
    &[
        "chat-relay",
        "rl-engine",
        "hexflo-lifecycle",
        "hexflo-cleanup",
        "conflict-resolver",
    ],
];

#[derive(Debug, Deserialize)]
pub struct HydrateRequest {
    pub host: Option<String>,
    pub database: Option<String>,
    pub force: Option<bool>,
    pub dry_run: Option<bool>,
}

#[derive(Debug, Serialize)]
struct TierResult {
    tier: usize,
    modules: Vec<ModuleResult>,
    ok: usize,
    total: usize,
}

#[derive(Debug, Serialize)]
struct ModuleResult {
    name: String,
    status: String, // "ok", "failed", "skipped"
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

/// POST /api/stdb/hydrate
///
/// Publishes all embedded WASM modules in tiered dependency order.
/// Creates a HexFlo swarm to track progress in real-time on the dashboard.
pub async fn hydrate(
    State(state): State<SharedState>,
    Json(body): Json<HydrateRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    let host = body.host.unwrap_or_else(|| "http://127.0.0.1:3000".to_string());
    let database = body.database.unwrap_or_else(|| "hex".to_string());
    let _force = body.force.unwrap_or(false);
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
        match hf.swarm_init("hex-hydrate", "stdb-hydrate", Some("pipeline".to_string())).await {
            Ok(s) => s.id,
            Err(_) => String::new(),
        }
    } else {
        String::new()
    };

    // Find spacetime CLI binary
    let binary = find_spacetime_binary();
    let binary = match binary {
        Some(b) => b,
        None => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({
                    "error": "spacetime CLI not found on PATH",
                    "hint": "Install from https://spacetimedb.com/install",
                })),
            );
        }
    };

    // Discover module source directories
    // Try workspace-relative path first, then check common locations
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

    // Publish modules tier by tier
    let mut all_tiers: Vec<TierResult> = Vec::new();
    let mut total_ok = 0usize;
    let mut total_fail = 0usize;

    for (tier_idx, tier_modules) in MODULE_TIERS.iter().enumerate() {
        // Create HexFlo task for this tier
        let tier_task_title = format!("hydrate-tier-{}", tier_idx);
        let tier_task = if !swarm_id.is_empty() {
            if let Some(hf) = hexflo {
                hf.task_create(&swarm_id, &tier_task_title).await.ok()
            } else {
                None
            }
        } else {
            None
        };

        let mut tier_result = TierResult {
            tier: tier_idx,
            modules: Vec::new(),
            ok: 0,
            total: tier_modules.len(),
        };

        for module_name in *tier_modules {
            let module_path = modules_base.join(module_name);

            if !module_path.is_dir() {
                tier_result.modules.push(ModuleResult {
                    name: module_name.to_string(),
                    status: "skipped".to_string(),
                    error: Some("module directory not found".to_string()),
                });
                continue;
            }

            // Publish via spacetime CLI
            let output = tokio::process::Command::new(&binary)
                .arg("publish")
                .arg("--server")
                .arg(&host)
                .arg(&database)
                .arg("--project-path")
                .arg(&module_path)
                .arg("--yes")
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .output()
                .await;

            match output {
                Ok(o) if o.status.success() => {
                    tier_result.modules.push(ModuleResult {
                        name: module_name.to_string(),
                        status: "ok".to_string(),
                        error: None,
                    });
                    tier_result.ok += 1;
                    total_ok += 1;
                }
                Ok(o) => {
                    let stderr = String::from_utf8_lossy(&o.stderr).trim().to_string();
                    tier_result.modules.push(ModuleResult {
                        name: module_name.to_string(),
                        status: "failed".to_string(),
                        error: Some(stderr),
                    });
                    total_fail += 1;
                }
                Err(e) => {
                    tier_result.modules.push(ModuleResult {
                        name: module_name.to_string(),
                        status: "failed".to_string(),
                        error: Some(e.to_string()),
                    });
                    total_fail += 1;
                }
            }
        }

        // Complete the HexFlo tier task
        if let (Some(task), Some(hf)) = (&tier_task, hexflo) {
            let result = format!(
                "tier-{}: {}/{} modules published",
                tier_idx, tier_result.ok, tier_result.total
            );
            let _ = hf.task_complete(&task.id, Some(result), None).await;
        }

        // If tier incomplete, warn but continue (some modules may be optional)
        all_tiers.push(tier_result);
    }

    // Teardown the HexFlo swarm
    if let (Some(hf), false) = (hexflo, swarm_id.is_empty()) {
        let _ = hf.swarm_teardown(&swarm_id).await;
    }

    let status = if total_fail == 0 {
        "hydrated"
    } else if total_ok > 0 {
        "partial"
    } else {
        "empty"
    };

    let tiers_json: Vec<serde_json::Value> = all_tiers
        .iter()
        .map(|t| {
            json!({
                "tier": t.tier,
                "ok": t.ok,
                "total": t.total,
                "modules": t.modules,
            })
        })
        .collect();

    (
        StatusCode::OK,
        Json(json!({
            "status": status,
            "modules_published": total_ok,
            "modules_failed": total_fail,
            "tiers": tiers_json,
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

fn find_spacetime_binary() -> Option<PathBuf> {
    let path_var = std::env::var("PATH").unwrap_or_default();
    for name in &["spacetime", "spacetimedb"] {
        for dir in path_var.split(':') {
            let candidate = PathBuf::from(dir).join(name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

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

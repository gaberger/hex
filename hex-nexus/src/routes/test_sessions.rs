//! REST endpoints for test session recording and querying.
//!
//! Since SpacetimeDB client bindings for the test-results module may not exist,
//! this uses a file-based fallback at ~/.hex/test-sessions/ storing JSON files.

use axum::{
    extract::{Path, Query, State},
    Json,
};
use http::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::path::PathBuf;

use crate::state::SharedState;

// ── Storage directory ─────────────────────────────────

fn sessions_dir() -> PathBuf {
    let base = std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/tmp"));
    base.join(".hex").join("test-sessions")
}

fn ensure_dir() -> std::io::Result<PathBuf> {
    let dir = sessions_dir();
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

// ── Types ─────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestResultEntry {
    pub id: String,
    pub category: String,
    pub name: String,
    pub status: String,
    #[serde(default)]
    pub duration_ms: u64,
    #[serde(default)]
    pub error_message: String,
    #[serde(default)]
    pub file_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestSessionRecord {
    pub id: String,
    #[serde(default)]
    pub agent_id: String,
    #[serde(default)]
    pub commit_hash: String,
    #[serde(default)]
    pub branch: String,
    pub started_at: String,
    #[serde(default)]
    pub finished_at: String,
    #[serde(default)]
    pub trigger: String,
    #[serde(default)]
    pub overall_status: String,
    #[serde(default)]
    pub pass_count: u32,
    #[serde(default)]
    pub fail_count: u32,
    #[serde(default)]
    pub skip_count: u32,
    #[serde(default)]
    pub total_count: u32,
    #[serde(default)]
    pub duration_ms: u64,
    #[serde(default)]
    pub results: Vec<TestResultEntry>,
}

/// Summary view of a session (without individual results).
#[derive(Debug, Serialize)]
struct SessionSummary {
    id: String,
    agent_id: String,
    commit_hash: String,
    branch: String,
    started_at: String,
    finished_at: String,
    trigger: String,
    overall_status: String,
    pass_count: u32,
    fail_count: u32,
    skip_count: u32,
    total_count: u32,
    duration_ms: u64,
}

impl From<&TestSessionRecord> for SessionSummary {
    fn from(s: &TestSessionRecord) -> Self {
        Self {
            id: s.id.clone(),
            agent_id: s.agent_id.clone(),
            commit_hash: s.commit_hash.clone(),
            branch: s.branch.clone(),
            started_at: s.started_at.clone(),
            finished_at: s.finished_at.clone(),
            trigger: s.trigger.clone(),
            overall_status: s.overall_status.clone(),
            pass_count: s.pass_count,
            fail_count: s.fail_count,
            skip_count: s.skip_count,
            total_count: s.total_count,
            duration_ms: s.duration_ms,
        }
    }
}

// ── Helpers ───────────────────────────────────────────

fn load_all_sessions() -> Vec<TestSessionRecord> {
    let dir = sessions_dir();
    if !dir.is_dir() {
        return Vec::new();
    }

    let mut sessions = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            if let Ok(content) = std::fs::read_to_string(&path) {
                if let Ok(session) = serde_json::from_str::<TestSessionRecord>(&content) {
                    sessions.push(session);
                }
            }
        }
    }

    // Sort by started_at descending
    sessions.sort_by(|a, b| b.started_at.cmp(&a.started_at));
    sessions
}

fn save_session(session: &TestSessionRecord) -> Result<(), String> {
    let dir = ensure_dir().map_err(|e| format!("Failed to create sessions dir: {}", e))?;
    let path = dir.join(format!("{}.json", session.id));
    let content = serde_json::to_string_pretty(session)
        .map_err(|e| format!("Failed to serialize session: {}", e))?;
    std::fs::write(&path, content)
        .map_err(|e| format!("Failed to write session file: {}", e))?;
    Ok(())
}

// ── Endpoints ─────────────────────────────────────────

/// POST /api/test-sessions — record a test session with nested results
pub async fn record(
    State(_state): State<SharedState>,
    Json(body): Json<TestSessionRecord>,
) -> (StatusCode, Json<serde_json::Value>) {
    let session_id = if body.id.is_empty() {
        uuid::Uuid::new_v4().to_string()
    } else {
        body.id.clone()
    };

    let session = TestSessionRecord {
        id: session_id.clone(),
        ..body
    };

    match save_session(&session) {
        Ok(()) => (
            StatusCode::OK,
            Json(json!({
                "ok": true,
                "session_id": session_id,
            })),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "ok": false, "error": e })),
        ),
    }
}

#[derive(Debug, Deserialize)]
pub struct ListQuery {
    pub limit: Option<usize>,
    pub branch: Option<String>,
}

/// GET /api/test-sessions — list recent sessions (summary only, no results)
pub async fn list(
    State(_state): State<SharedState>,
    Query(query): Query<ListQuery>,
) -> (StatusCode, Json<serde_json::Value>) {
    let limit = query.limit.unwrap_or(10).min(50);
    let sessions = load_all_sessions();

    let filtered: Vec<SessionSummary> = sessions
        .iter()
        .filter(|s| {
            if let Some(ref branch) = query.branch {
                s.branch == *branch
            } else {
                true
            }
        })
        .take(limit)
        .map(SessionSummary::from)
        .collect();

    (
        StatusCode::OK,
        Json(json!({
            "ok": true,
            "count": filtered.len(),
            "sessions": filtered,
        })),
    )
}

/// GET /api/test-sessions/:id — get full session with results
pub async fn get_session(
    State(_state): State<SharedState>,
    Path(id): Path<String>,
) -> (StatusCode, Json<serde_json::Value>) {
    let dir = sessions_dir();
    let path = dir.join(format!("{}.json", id));

    if !path.exists() {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({ "ok": false, "error": "Session not found" })),
        );
    }

    match std::fs::read_to_string(&path) {
        Ok(content) => match serde_json::from_str::<TestSessionRecord>(&content) {
            Ok(session) => (StatusCode::OK, Json(json!({ "ok": true, "session": session }))),
            Err(e) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "ok": false, "error": format!("Failed to parse session: {}", e) })),
            ),
        },
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "ok": false, "error": format!("Failed to read session: {}", e) })),
        ),
    }
}

#[derive(Debug, Deserialize)]
pub struct TrendsQuery {
    pub runs: Option<usize>,
}

/// GET /api/test-sessions/flaky — detect flaky tests across recent sessions
pub async fn flaky(
    State(_state): State<SharedState>,
    Query(query): Query<TrendsQuery>,
) -> (StatusCode, Json<serde_json::Value>) {
    let runs = query.runs.unwrap_or(10).min(50);
    let sessions = load_all_sessions();
    let recent: Vec<&TestSessionRecord> = sessions.iter().take(runs).collect();

    // Collect per-test pass/fail counts across sessions
    let mut test_stats: std::collections::HashMap<String, (String, u32, u32)> =
        std::collections::HashMap::new();

    for session in &recent {
        for result in &session.results {
            let key = format!("{}::{}", result.category, result.name);
            let entry = test_stats
                .entry(key)
                .or_insert_with(|| (result.category.clone(), 0, 0));
            match result.status.as_str() {
                "pass" | "passed" => entry.1 += 1,
                "fail" | "failed" => entry.2 += 1,
                _ => {}
            }
        }
    }

    // A test is flaky if it has both passes and failures
    let mut flaky_tests: Vec<serde_json::Value> = test_stats
        .iter()
        .filter(|(_, (_, pass, fail))| *pass > 0 && *fail > 0)
        .map(|(key, (category, pass, fail))| {
            let name = key
                .strip_prefix(&format!("{}::", category))
                .unwrap_or(key);
            let total = *pass + *fail;
            let flake_rate = *fail as f64 / total as f64;
            json!({
                "name": name,
                "category": category,
                "pass_count": pass,
                "fail_count": fail,
                "flake_rate": (flake_rate * 1000.0).round() / 1000.0,
            })
        })
        .collect();

    // Sort by flake_rate descending
    flaky_tests.sort_by(|a, b| {
        let ra = a["flake_rate"].as_f64().unwrap_or(0.0);
        let rb = b["flake_rate"].as_f64().unwrap_or(0.0);
        rb.partial_cmp(&ra).unwrap_or(std::cmp::Ordering::Equal)
    });

    (
        StatusCode::OK,
        Json(json!({
            "ok": true,
            "sessions_analyzed": recent.len(),
            "flaky_tests": flaky_tests,
        })),
    )
}

/// GET /api/test-sessions/trends — pass rate trends per category
pub async fn trends(
    State(_state): State<SharedState>,
    Query(query): Query<TrendsQuery>,
) -> (StatusCode, Json<serde_json::Value>) {
    let runs = query.runs.unwrap_or(10).min(50);
    let sessions = load_all_sessions();
    let recent: Vec<&TestSessionRecord> = sessions.iter().take(runs).collect();

    // Build per-category pass/fail arrays (oldest first for chronological order)
    let mut categories: std::collections::HashMap<String, Vec<bool>> =
        std::collections::HashMap::new();

    // Iterate in reverse so index 0 = oldest session
    for session in recent.iter().rev() {
        // Group results by category within this session
        let mut cat_results: std::collections::HashMap<String, (u32, u32)> =
            std::collections::HashMap::new();

        for result in &session.results {
            let entry = cat_results.entry(result.category.clone()).or_insert((0, 0));
            if result.status == "pass" || result.status == "passed" {
                entry.0 += 1;
            } else if result.status == "fail" || result.status == "failed" {
                entry.1 += 1;
            }
        }

        // If no individual results, use session-level data with "all" category
        if cat_results.is_empty() {
            let passed = session.fail_count == 0 && session.total_count > 0;
            categories
                .entry("all".to_string())
                .or_default()
                .push(passed);
        } else {
            // For each category, session passes if no failures in that category
            for (cat, (pass, fail)) in &cat_results {
                let all_passed = *fail == 0 && *pass > 0;
                categories.entry(cat.clone()).or_default().push(all_passed);
            }
        }
    }

    (
        StatusCode::OK,
        Json(json!({
            "ok": true,
            "runs": recent.len(),
            "categories": categories,
        })),
    )
}

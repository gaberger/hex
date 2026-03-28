//! sandbox-demo — a simple key-value store built inside a hex Docker sandbox.
//!
//! Built by hex-agent running in a Docker AI Sandbox microVM (ADR-2603282000).
//! The agent received the task via HexFlo, wrote this file via hex_write_file,
//! verified with hex_bash("cargo check"), then committed via hex_git_commit.
//!
//! Run:  cargo run
//! Test: cargo test
//! API:  http://localhost:3030

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::Json,
    routing::{delete, get, post},
    Router,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ── Domain ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entry {
    pub id: String,
    pub key: String,
    pub value: String,
    pub created_at: String,
}

#[derive(Debug, Deserialize)]
pub struct CreateEntry {
    pub key: String,
    pub value: String,
}

// ── State ─────────────────────────────────────────────────────────────────────

type Store = Arc<Mutex<HashMap<String, Entry>>>;

// ── Handlers ──────────────────────────────────────────────────────────────────

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({"status": "ok", "service": "sandbox-demo"}))
}

async fn list_entries(State(store): State<Store>) -> Json<Vec<Entry>> {
    let entries = store.lock().unwrap();
    let mut list: Vec<Entry> = entries.values().cloned().collect();
    list.sort_by(|a, b| a.created_at.cmp(&b.created_at));
    Json(list)
}

async fn create_entry(
    State(store): State<Store>,
    Json(body): Json<CreateEntry>,
) -> (StatusCode, Json<Entry>) {
    let id = Uuid::new_v4().to_string();
    let created_at = chrono_now();
    let entry = Entry {
        id: id.clone(),
        key: body.key,
        value: body.value,
        created_at,
    };
    store.lock().unwrap().insert(id, entry.clone());
    (StatusCode::CREATED, Json(entry))
}

async fn get_entry(
    State(store): State<Store>,
    Path(id): Path<String>,
) -> Result<Json<Entry>, StatusCode> {
    let entries = store.lock().unwrap();
    entries
        .get(&id)
        .cloned()
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

async fn delete_entry(
    State(store): State<Store>,
    Path(id): Path<String>,
) -> StatusCode {
    let mut entries = store.lock().unwrap();
    if entries.remove(&id).is_some() {
        StatusCode::NO_CONTENT
    } else {
        StatusCode::NOT_FOUND
    }
}

async fn get_by_key(
    State(store): State<Store>,
    Path(key): Path<String>,
) -> Result<Json<Entry>, StatusCode> {
    let entries = store.lock().unwrap();
    entries
        .values()
        .find(|e| e.key == key)
        .cloned()
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

// ── Router ────────────────────────────────────────────────────────────────────

pub fn router(store: Store) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/entries", get(list_entries).post(create_entry))
        .route("/entries/{id}", get(get_entry).delete(delete_entry))
        .route("/entries/by-key/{key}", get(get_by_key))
        .with_state(store)
}

// ── Main ──────────────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() {
    let store: Store = Arc::new(Mutex::new(HashMap::new()));
    let port = std::env::var("PORT").unwrap_or_else(|_| "3030".into());
    let addr = format!("0.0.0.0:{port}");

    println!("sandbox-demo listening on http://{addr}");
    let listener = tokio::net::TcpListener::bind(&addr).await.unwrap();
    axum::serve(listener, router(store)).await.unwrap();
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn chrono_now() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    // ISO 8601 without chrono dependency: YYYY-MM-DDTHH:MM:SSZ
    let (y, mo, d, h, mi, s) = epoch_to_parts(secs);
    format!("{y:04}-{mo:02}-{d:02}T{h:02}:{mi:02}:{s:02}Z")
}

fn epoch_to_parts(secs: u64) -> (u64, u64, u64, u64, u64, u64) {
    let s = secs % 60;
    let m = (secs / 60) % 60;
    let h = (secs / 3600) % 24;
    let days = secs / 86400;
    // Gregorian calendar approximation (good through ~2100)
    let year = 1970 + days / 365;
    let yd = days % 365;
    let month = yd / 30 + 1;
    let day = yd % 30 + 1;
    (year, month.min(12), day.min(28), h, m, s)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt; // for .oneshot()

    fn test_store() -> Store {
        Arc::new(Mutex::new(HashMap::new()))
    }

    #[tokio::test]
    async fn health_returns_ok() {
        let app = router(test_store());
        let resp = app
            .oneshot(Request::builder().uri("/health").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn create_then_get_entry() {
        let store = test_store();
        let app = router(store.clone());

        // Create
        let body = serde_json::json!({"key": "greeting", "value": "hello"}).to_string();
        let req = Request::builder()
            .method("POST")
            .uri("/entries")
            .header("content-type", "application/json")
            .body(Body::from(body))
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);

        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let entry: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        let id = entry["id"].as_str().unwrap().to_string();

        // Get by ID
        let resp2 = router(store)
            .oneshot(
                Request::builder()
                    .uri(format!("/entries/{id}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp2.status(), StatusCode::OK);
        let bytes2 = axum::body::to_bytes(resp2.into_body(), usize::MAX).await.unwrap();
        let fetched: serde_json::Value = serde_json::from_slice(&bytes2).unwrap();
        assert_eq!(fetched["value"], "hello");
    }

    #[tokio::test]
    async fn list_entries_initially_empty() {
        let app = router(test_store());
        let resp = app
            .oneshot(Request::builder().uri("/entries").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let list: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(list.as_array().unwrap().len(), 0);
    }

    #[tokio::test]
    async fn delete_entry() {
        let store = test_store();

        // Create
        let body = serde_json::json!({"key": "tmp", "value": "to-delete"}).to_string();
        let create_resp = router(store.clone())
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/entries")
                    .header("content-type", "application/json")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        let bytes = axum::body::to_bytes(create_resp.into_body(), usize::MAX).await.unwrap();
        let entry: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        let id = entry["id"].as_str().unwrap().to_string();

        // Delete
        let del_resp = router(store)
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri(format!("/entries/{id}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(del_resp.status(), StatusCode::NO_CONTENT);
    }

    #[tokio::test]
    async fn get_nonexistent_entry_returns_404() {
        let app = router(test_store());
        let req = Request::builder()
            .uri("/entries/does-not-exist")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }
}

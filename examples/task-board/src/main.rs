//! Real-time collaborative task board — axum + WebSockets + in-memory state.
//!
//! REST API:
//!   GET    /tasks           — list all tasks
//!   POST   /tasks           — create a task  { "title": "..." }
//!   PATCH  /tasks/:id       — update a task  { "title"?, "done"? }
//!   DELETE /tasks/:id       — delete a task
//!
//! WebSocket:
//!   GET    /ws              — subscribe; receive JSON events for every change
//!
//! All mutating REST calls broadcast the updated task list to every
//! connected WebSocket client so all browsers stay in sync instantly.

use std::collections::HashMap;
use std::sync::Arc;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, patch};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use tokio::sync::{broadcast, RwLock};
use tower_http::cors::CorsLayer;
use uuid::Uuid;

// ── Domain ────────────────────────────────────────────────────────────────────

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Task {
    id: String,
    title: String,
    done: bool,
}

#[derive(Debug, Deserialize)]
struct CreateTask {
    title: String,
}

#[derive(Debug, Deserialize)]
struct UpdateTask {
    title: Option<String>,
    done: Option<bool>,
}

// ── App state ─────────────────────────────────────────────────────────────────

#[derive(Clone)]
struct AppState {
    tasks: Arc<RwLock<HashMap<String, Task>>>,
    broadcast: broadcast::Sender<String>,
}

impl AppState {
    fn new() -> Self {
        let (tx, _) = broadcast::channel(64);
        Self {
            tasks: Arc::new(RwLock::new(HashMap::new())),
            broadcast: tx,
        }
    }

    async fn snapshot_json(&self) -> String {
        let tasks = self.tasks.read().await;
        let mut list: Vec<&Task> = tasks.values().collect();
        list.sort_by(|a, b| a.id.cmp(&b.id));
        serde_json::to_string(&list).unwrap_or_default()
    }

    fn publish(&self, snapshot: String) {
        // Ignore send errors when there are no subscribers.
        let _ = self.broadcast.send(snapshot);
    }
}

// ── Handlers ──────────────────────────────────────────────────────────────────

async fn list_tasks(State(state): State<AppState>) -> Json<Vec<Task>> {
    let tasks = state.tasks.read().await;
    let mut list: Vec<Task> = tasks.values().cloned().collect();
    list.sort_by(|a, b| a.id.cmp(&b.id));
    Json(list)
}

async fn create_task(
    State(state): State<AppState>,
    Json(body): Json<CreateTask>,
) -> impl IntoResponse {
    let task = Task {
        id: Uuid::new_v4().to_string(),
        title: body.title,
        done: false,
    };
    state.tasks.write().await.insert(task.id.clone(), task.clone());
    let snap = state.snapshot_json().await;
    state.publish(snap);
    (StatusCode::CREATED, Json(task))
}

async fn update_task(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<UpdateTask>,
) -> impl IntoResponse {
    let mut tasks = state.tasks.write().await;
    match tasks.get_mut(&id) {
        None => StatusCode::NOT_FOUND.into_response(),
        Some(task) => {
            if let Some(title) = body.title {
                task.title = title;
            }
            if let Some(done) = body.done {
                task.done = done;
            }
            let updated = task.clone();
            drop(tasks);
            let snap = state.snapshot_json().await;
            state.publish(snap);
            Json(updated).into_response()
        }
    }
}

async fn delete_task(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let removed = state.tasks.write().await.remove(&id).is_some();
    if removed {
        let snap = state.snapshot_json().await;
        state.publish(snap);
        StatusCode::NO_CONTENT
    } else {
        StatusCode::NOT_FOUND
    }
}

// ── WebSocket ─────────────────────────────────────────────────────────────────

async fn ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_ws(socket, state))
}

async fn handle_ws(mut socket: WebSocket, state: AppState) {
    // Send current snapshot on connect.
    let snap = state.snapshot_json().await;
    if socket.send(Message::Text(snap.into())).await.is_err() {
        return;
    }

    let mut rx = state.broadcast.subscribe();

    loop {
        tokio::select! {
            Ok(event) = rx.recv() => {
                if socket.send(Message::Text(event.into())).await.is_err() {
                    break;
                }
            }
            msg = socket.recv() => {
                // Accept ping/close; ignore everything else.
                match msg {
                    Some(Ok(Message::Close(_))) | None => break,
                    _ => {}
                }
            }
        }
    }
}

// ── Main ──────────────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() {
    let state = AppState::new();

    let app = Router::new()
        .route("/tasks", get(list_tasks).post(create_task))
        .route("/tasks/:id", patch(update_task).delete(delete_task))
        .route("/ws", get(ws_handler))
        .layer(CorsLayer::permissive())
        .with_state(state);

    let addr = "0.0.0.0:3030";
    println!("task-board listening on http://{addr}");
    println!("  REST  : GET/POST /tasks  |  PATCH/DELETE /tasks/:id");
    println!("  WS    : ws://{addr}/ws");

    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}

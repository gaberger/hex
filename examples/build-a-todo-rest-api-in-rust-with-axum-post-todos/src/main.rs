use axum::{
    Router,
    routing::{get, delete},
    extract::{State, Path},
    http::StatusCode,
    serve,
    response::Json,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::net::TcpListener;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct TodoItem {
    id: usize,
    title: String,
    completed: bool,
}

#[derive(Debug, Serialize, Deserialize)]
struct CreateRequest {
    title: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct MyState {
    todos: Vec<TodoItem>,
}

#[tokio::main]
async fn main() {
    let state = Arc::new(Mutex::new(MyState {
        todos: Vec::new(),
    }));

    let app = Router::new()
        .route("/todos", get(list_todos))
        .route("/todos/:id", delete(delete_todo));

    let listener = TcpListener::bind("0.0.0.0:3000").await.unwrap();
    serve(listener, app).await.unwrap();
}

async fn list_todos(State(state): State<Arc<Mutex<MyState>>>) -> Json<Vec<TodoItem>> {
    let todos = state.lock().await.todos.clone();
    Json(todos)
}

async fn delete_todo(
    State(state): State<Arc<Mutex<MyState>>>,
    Path(id): Path<usize>,
) -> StatusCode {
    let mut state = state.lock().await;
    state.todos.retain(|t| t.id != id);
    StatusCode::NO_CONTENT
}
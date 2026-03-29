use axum::{
    Json,
    extract::{State, Path},
    routing::{get, post, delete},
    Router,
};
use axum::http::StatusCode;
use std::sync::Arc;
use tokio::sync::Mutex;

#[tokio::main]
async fn main() {
    let state: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let app = Router::new()
        .route("/todos", get(list_todos).post(create_todo))
        .route("/todos/{id}", delete(delete_todo))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await.unwrap();
    axum::serve(listener, app).await.unwrap();
}

async fn list_todos(State(state): State<Arc<Mutex<Vec<String>>>>) -> Json<Vec<String>> {
    Json(state.lock().await.clone())
}

async fn create_todo(
    State(state): State<Arc<Mutex<Vec<String>>>>,
    Json(todo): Json<String>,
) -> Json<String> {
    let mut inner = state.lock().await;
    inner.push(todo.clone());
    Json(todo)
}

async fn delete_todo(
    State(state): State<Arc<Mutex<Vec<String>>>>,
    Path(id): Path<String>,
) -> Json<String> {
    let mut inner = state.lock().await;
    inner.retain(|i| i != &id);
    Json(id)
}
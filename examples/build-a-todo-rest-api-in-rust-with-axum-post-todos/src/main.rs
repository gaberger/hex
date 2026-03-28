use std::sync::Arc;

use axum::{
    Router,
    routing::{get, post},
    extract::{State, Json},
    http::StatusCode,
    serve,
};

#[derive(Clone)]
struct AppState {
    message: String,
    version: u32,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct GreetRequest {
    name: String,
}

#[derive(serde::Serialize)]
struct GreetResponse {
    greeting: String,
    timestamp: String,
}

async fn get_message(State(state): State<AppState>) -> Json<String> {
    Json(state.message.clone())
}

async fn greet(State(state): State<AppState>, Json(request): Json<GreetRequest>) -> Json<GreetResponse> {
    let greeting = format!("Hello, {}! Version {}", request.name, state.version);
    Json(GreetResponse {
        greeting,
        timestamp: "2024-01-01".to_string(),
    })
}

async fn health(State(_state): State<AppState>) -> Json<String> {
    Json("OK".to_string())
}

#[tokio::main]
async fn main() {
    let app_state = Arc::new(AppState {
        message: "Welcome to the Hexagonal API".to_string(),
        version: 1,
    });

    let router = Router::new()
        .route("/message", get(get_message))
        .route("/greet", post(greet))
        .route("/health", get(health))
        .with_state(app_state);

    let listener = tokio::net::TcpListener::bind("0.0.0.0:8080").await.unwrap();
    println!("Server starting on 0.0.0.0:8080");
    serve(listener, router).await.unwrap();
}
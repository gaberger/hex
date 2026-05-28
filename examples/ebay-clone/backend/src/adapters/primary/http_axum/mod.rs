use std::sync::Arc;

use axum::{
    routing::{get, post},
    Router,
};
use tower_http::auth::RequireAuthorizationLayer;
use tracing::info;

use crate::{
    adapters::primary::http_axum::{auth_middleware::JwtAuthMiddleware, state::AppState},
    ports::{self},
};

// ADR-2026-05-19-0721
pub fn create_router(ports: Arc<Ports>) -> Router {
    let app_state = AppState { user_port: ports.user_port.clone() };

    Router::new()
        .route("/", get(root))
        .layer(JwtAuthMiddleware::from(app_state.clone()))
}

async fn root() -> &'static str {
    "Hello, world!"
}

#[derive(Clone)]
pub struct Ports {
    pub user_port: Arc<dyn ports::UserPort + Send + Sync>,
}

impl AppState {
    pub fn new(ports: Ports) -> Self {
        AppState { user_port: ports.user_port }
    }
}
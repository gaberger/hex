use std::sync::Arc;

use axum::{
    routing::{get, post},
    Router,
};
use tower_http::auth::RequireAuthorizationLayer;
use tracing::info;

use crate::{
    adapters::primary::http_axum::{auth_middleware::JwtAuthMiddleware, state::AppState, handlers_bidding, handlers_me},
    ports::{self},
};

// ADR-2026-05-19-0721
pub fn create_router(ports: Arc<Ports>) -> Router {
    let app_state = AppState { user_port: ports.user_port.clone() };

    Router::new()
        .route("/", get(root))
        .route("/api/v1/listings/:id/bid", post(handlers_bidding::place_bid))
        .route("/api/v1/listings/:id/watch", post(handlers_bidding::toggle_watchlist))
        .route("/api/v1/me/bids", get(handlers_me::get_my_bids))
        .route("/api/v1/me/won", get(handlers_me::get_won_items))
        .route("/api/v1/me/listings", get(handlers_me::get_my_listings))
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
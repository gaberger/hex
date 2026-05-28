use std::sync::Arc;

use axum::{
    routing::{get, post},
    Router,
};
use tower_http::auth::RequireAuthorizationLayer;
use tracing::info;

use crate::{
    adapters::primary::http_axum::{auth_middleware::JwtAuthMiddleware, state::AppState, handlers_bidding, handlers_me, handlers_listings, handlers_images, handlers_auth},
    ports::{self},
};

// ADR-2026-05-19-0721
pub fn create_router(ports: Arc<Ports>) -> Router {
    let app_state = AppState { user_port: ports.user_port.clone() };

    Router::new()
        .route("/", get(root))
        .route("/api/v1/listings", post(handlers_listings::create_listing).get(handlers_listings::search_listings))
        .route("/api/v1/listings/:id", get(handlers_listings::get_listing_by_id))
        .route("/api/v1/listings/:id/bid", post(handlers_bidding::place_bid))
        .route("/api/v1/listings/:id/watch", post(handlers_bidding::toggle_watchlist))
        .route("/api/v1/me/bids", get(handlers_me::get_my_bids))
        .route("/api/v1/me/won", get(handlers_me::get_won_items))
        .route("/api/v1/me/listings", get(handlers_me::get_my_listings))
        .route("/api/v1/images", post(handlers_images::upload_image))
        .route("/api/v1/auth/register", post(handlers_auth::register_user))
        .route("/api/v1/auth/login", post(handlers_auth::login_user))
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
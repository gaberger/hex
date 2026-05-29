use std::sync::Arc;

use axum::{
    routing::{get, post},
    Router,
};
use tracing::info;
use axum::middleware::from_fn;

use crate::{
    adapters::primary::http_axum::{
        auth_middleware,
        dto::{UserRequest, UserResponse, ItemRequest, ItemResponse, BidRequest, BidResponse},
        handlers_auth::auth_routes,
        handlers_bidding::{place_bid, toggle_watch},
        handlers_images::routes as images_routes,
        handlers_listings::{
            ListingResponse, CreateListingRequest, SearchListingsParams, create_listing, get_listings,
            get_listing_by_id, listings_routes,
        },
        handlers_me::{get_my_bids, get_my_won_items, get_my_listings},
        state::AppState,
    },
    adapters::secondary::stdb_client::connect,
    core::ports::user_repo::UserRepoPort,
};

// ADR-2026-05-19-0721
pub fn create_router(ports: Arc<Ports>) -> Router {
    let app_state = AppState { user_port: ports.user_port.clone() };

    Router::new()
        .route("/", get(root))
        .nest("/api/v1/listings", listings_routes())
        .route("/api/v1/me/bids", get(get_my_bids))
        .route("/api/v1/me/won", get(get_my_won_items))
        .route("/api/v1/me/listings", get(get_my_listings))
        .nest("/api/v1/images", images_routes())
        .nest("/api/v1/auth", auth_routes())
        .layer(from_fn(auth_middleware))
}

async fn root() -> &'static str {
    "Hello, world!"
}

#[derive(Clone)]
pub struct Ports {
    pub user_port: Arc<dyn UserRepoPort + Send + Sync>,
}

impl AppState {
    pub fn new(user_port: Arc<dyn UserRepoPort + Send + Sync>) -> Self {
        AppState { user_port }
    }
}
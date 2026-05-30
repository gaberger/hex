use std::sync::Arc;

use axum::{routing::get, Router};

use crate::core::ports::user_repo::UserRepoPort;

// ADR-2026-05-19-0721
//
// HTTP primary adapter. The per-resource handler modules (`dto`, `state`,
// `auth_middleware`, `handlers_auth`, `handlers_listings`, `handlers_bidding`,
// `handlers_images`, `handlers_me`) and the `stdb_client::connection` submodule
// are not part of this repair cluster, so importing them produced unresolved
// imports (E0432). The router currently exposes only the health-check root;
// each route is wired back in as its handler module lands. `AppState`/`Ports`
// are defined here (rather than imported from a missing `state` module) so the
// composition root can construct the adapter.

#[derive(Clone)]
pub struct AppState {
    pub user_port: Arc<dyn UserRepoPort + Send + Sync>,
}

impl AppState {
    pub fn new(user_port: Arc<dyn UserRepoPort + Send + Sync>) -> Self {
        AppState { user_port }
    }
}

#[derive(Clone)]
pub struct Ports {
    pub user_port: Arc<dyn UserRepoPort + Send + Sync>,
}

pub fn create_router(ports: Arc<Ports>) -> Router {
    let _app_state = AppState::new(ports.user_port.clone());

    Router::new().route("/", get(root))
}

async fn root() -> &'static str {
    "Hello, world!"
}
use crate::adapters::{StdDbClient, FsImageStore, Argon2Hasher, JwtSigner, SystemClock};
use axum::Router;
use std::sync::Arc;

/// Composition root for the eBay clone backend.
///
/// This file is responsible for:
/// - Reading environment variables.
/// - Instantiating concrete adapters.
/// - Boxing each adapter as `Arc<dyn Port>`.
/// - Constructing `AppState`.
/// - Building the axum `Router`.
/// - Spawning the server on 0.0.0.0:8080.
///
/// It is the only file in the backend that is permitted to import from `crate::adapters::*` modules.
///
/// Refer to:
/// - docs/specs/ebay-spec-023
/// - docs/specs/ebay-spec-024
/// - ADR-2026-05-19-0721
pub fn compose_app() -> (Router, AppState) {
    // Read environment variables.
    let database_url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set");
    let jwt_secret = std::env::var("JWT_SECRET").expect("JWT_SECRET must be set");
    let image_root = std::env::var("IMAGE_ROOT").expect("IMAGE_ROOT must be set");
    let backend_addr = std::env::var("BACKEND_ADDR").unwrap_or_else(|_| "0.0.0.0:8080".to_string());

    // Instantiate concrete adapters.
    let db_client = StdDbClient::new(&database_url).expect("Failed to create database client");
    let image_store = FsImageStore::new(image_root);
    let hasher = Argon2Hasher;
    let signer = JwtSigner::new(jwt_secret);
    let clock = SystemClock;

    // Box each adapter as `Arc<dyn Port>`.
    let db_client: Arc<dyn DbPort> = Arc::new(db_client);
    let image_store: Arc<dyn ImageStorePort> = Arc::new(image_store);
    let hasher: Arc<dyn HasherPort> = Arc::new(hasher);
    let signer: Arc<dyn SignerPort> = Arc::new(signer);
    let clock: Arc<dyn ClockPort> = Arc::new(clock);

    // Construct AppState.
    let app_state = AppState {
        db_client,
        image_store,
        hasher,
        signer,
        clock,
    };

    // Build the axum Router.
    let router = build_router(&app_state);

    (router, app_state)
}

/// Constructs the axum Router using the provided `AppState`.
fn build_router(app_state: &AppState) -> Router {
    use crate::primary_adapter::api_v1::health::routes;

    Router::new()
        .merge(routes())
        // Additional routes can be merged here.
        .with_state(app_state.clone())
}

/// Application state that holds all the required services.
pub struct AppState {
    pub db_client: Arc<dyn DbPort>,
    pub image_store: Arc<dyn ImageStorePort>,
    pub hasher: Arc<dyn HasherPort>,
    pub signer: Arc<dyn SignerPort>,
    pub clock: Arc<dyn ClockPort>,
}

// Define trait aliases for clarity and to avoid repetition.
pub type DbPort = crate::adapters::DbPort;
pub type ImageStorePort = crate::adapters::ImageStorePort;
pub type HasherPort = crate::adapters::HasherPort;
pub type SignerPort = crate::adapters::SignerPort;
pub type ClockPort = crate::adapters::ClockPort;
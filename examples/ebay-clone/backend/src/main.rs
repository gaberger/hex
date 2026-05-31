//! eBay-clone backend binary.
//!
//! Thin entry point: builds the fully-wired router from the composition root,
//! seeds a demo catalog so the app launches populated, and serves it. All
//! wiring lives in the library crate so it can be exercised directly by the
//! acceptance test (which uses the unseeded `compose_app`).

use ebay_clone_backend::composition_root::{compose_parts, seed_demo_data};
use tokio::net::TcpListener;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .init();

    let (market, app) = compose_parts();
    seed_demo_data(&market).await;
    tracing::info!("seeded demo catalog");

    let addr = std::env::var("BACKEND_ADDR").unwrap_or_else(|_| "0.0.0.0:8080".to_string());
    let listener = TcpListener::bind(&addr)
        .await
        .expect("failed to bind BACKEND_ADDR");
    tracing::info!("ebay-clone backend listening on {addr}");

    axum::serve(listener, app.into_make_service())
        .await
        .expect("server error");
}

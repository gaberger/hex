// SPDX-License-Identifier: MIT
// ADR-2026-05-20-0800: eBay MVP Backend Skeleton
// This file serves as the entry point for the ebay-clone-backend crate.
// It implements the initial module structure required for step-1 of the workplan
// located at docs/workplans/feat-ebay-mvp.json.
//
// The backend architecture is inspired by the hex-nexus/ports-adapters pattern
// to ensure separation of concerns between domain logic and infrastructure.
// References:
// - docs/specs/ebay-mvp.json (specs ebay-spec-023, ebay-spec-024)
// - hex-cli/ for project scaffolding conventions

mod core;
mod adapters;
use composition_root::create_app_state;
use std::sync::Arc;
use axum::{Router, routing::get};
use tokio::net::TcpListener;

#[tokio::main]
async fn main() {
    // Initialize tracing for observability, consistent with hex-core/ standards
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .init();

    tracing::info!("Ebay Clone Backend is starting up!");

    let app_state = create_app_state();
    let app = Router::new()
        .route("/api/v1/health", get(|| async { "OK" }))
        // Add other routes here from primary adapter step-12
        .with_state(app_state);

    let addr = std::env::var("BACKEND_ADDR").unwrap_or_else(|_| "0.0.0.0:8080".to_string());
    let listener = TcpListener::bind(&addr).await.unwrap();
    tracing::info!("Listening on {}", addr);
    axum::Server::from_tcp(listener)
        .unwrap()
        .serve(app.into_make_service())
        .await
        .unwrap();
}
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

use ebay_clone_backend::app;

#[tokio::main]
async fn main() {
    // Initialize tracing for observability, consistent with hex-core/ standards
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .init();

    tracing::info!("Ebay Clone Backend is starting up!");

    // In this skeleton phase, we verify the module structure compiles.
    // Subsequent steps will implement the actual application logic here.
    
    match app::run().await {
        Ok(_) => {
            tracing::info!("Application finished successfully.");
        }
        Err(e) => {
            tracing::error!("Application failed: {}", e);
            std::process::exit(1);
        }
    }
}
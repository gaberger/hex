// Placeholder module for primary adapters

// ADR-2026-05-19-0721: Define primary adapter interfaces and implementations for ebay-clone
// Per feat-ebay-mvp step-1, we establish the module skeleton without business logic.
// This allows `cargo check -p ebay-clone-backend` to succeed while subsequent steps
// fill in the specific HTTP/CLI/GraphQL implementations referenced in docs/specs/ebay-mvp.json.

pub mod http_axum; // Added for feat-ebay-mvp step-12
pub mod cli;
pub mod graphql;

// Additional primary adapters can be added here as needed

// docs/workplans/feat-ebay-mvp.json
// Per spec conventions, the `http_axum` module will contain:
// - Router builder
// - Shared AppState struct (holds Arc<dyn Port> for each port the handlers need)
// - Error mapping module (DomainError -> StatusCode)
// - JSON request/response DTOs
// - JWT extractor middleware that verifies via TokenIssuerPort

pub use http_axum::{AppState, router, auth_middleware}; // Added for feat-ebay-mvp step-12
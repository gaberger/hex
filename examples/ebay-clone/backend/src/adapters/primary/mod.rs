// Placeholder module for primary adapters

// ADR-2026-05-19-0721: Define primary adapter interfaces and implementations for ebay-clone
// Per feat-ebay-mvp step-1, we establish the module skeleton without business logic.
// This allows `cargo check -p ebay-clone-backend` to succeed while subsequent steps
// fill in the specific HTTP/CLI/GraphQL implementations referenced in docs/specs/ebay-mvp.json.

pub mod http_axum; // Added for feat-ebay-mvp step-12

// Additional primary adapters can be added here as needed
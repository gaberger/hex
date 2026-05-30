//! HTTP primary adapter (axum).
//!
//! The coherent router + `AppState` live in [`app`]. The per-resource handler
//! files in this directory (`handlers_*.rs`, `state.rs`, `dto.rs`, ...) were
//! authored independently against three incompatible `AppState`/DI conventions
//! and are not part of the module tree; [`app`] is the single wired surface.

pub mod app;

pub use app::{build_router, AppState};

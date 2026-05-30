// Composition root for the eBay clone backend.
//
// This is the only file permitted to import from `crate::adapters::*`. As
// concrete adapters land they are instantiated here, boxed behind their port
// traits as declared in `crate::core::ports` (the real trait names are
// `ImageStorePort`, `ClockPort`, `PasswordHasherPort`, `TokenIssuerPort`,
// `UserRepoPort`, `ListingRepoPort`, `AuctionRepoPort`, `BidRepoPort`,
// `WatchRepoPort`, `ReducerCallPort`), and stored on `AppState`.
//
// The previous version referenced adapter structs (`StdDbClient`,
// `FsImageStore`, `Argon2Hasher`, `JwtSigner`, `SystemClock`) and port names
// (`DbPort`, `HasherPort`, `SignerPort`) that exist in neither the adapters
// module nor `core::ports`. Per hex rules the inner contracts are the source
// of truth, so this outer file is rewritten to conform: it no longer invents
// adapter/port names and wires only the framework-level state until adapters
// are implemented.
//
// Refer to:
// - docs/specs/ebay-spec-023
// - docs/specs/ebay-spec-024
// - ADR-2026-05-19-0721

use axum::routing::get;
use axum::Router;

/// Application state shared across all axum handlers.
///
/// Holds the boxed port implementations once concrete adapters are wired in.
/// Must be `Clone` because axum clones state per request.
#[derive(Clone, Default)]
pub struct AppState {}

/// Constructs the shared [`AppState`].
///
/// This is the single place that will instantiate concrete adapters and box
/// them behind their port traits from `crate::core::ports`.
pub fn create_app_state() -> AppState {
    AppState::default()
}

/// Builds the axum [`Router`] together with its [`AppState`].
pub fn compose_app() -> (Router, AppState) {
    let app_state = create_app_state();
    let router = Router::new()
        .route("/api/v1/health", get(|| async { "OK" }))
        .with_state(app_state.clone());
    (router, app_state)
}
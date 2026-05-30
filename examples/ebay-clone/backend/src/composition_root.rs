//! Composition root for the eBay-clone backend.
//!
//! The ONLY module permitted to import from `crate::adapters::*`. It instantiates
//! the concrete adapters, boxes them behind their port traits, constructs the
//! application use cases, and hands the assembled [`AppState`] to the primary
//! adapter's router builder.
//!
//! Profile: in-memory. [`InMemoryMarketplace`] stands in for the SpacetimeDB
//! `marketplace` module, satisfying the same port contracts. Swapping in the
//! real `stdb_client` adapter is a change confined to this file.

use std::sync::Arc;

use axum::Router;

use crate::adapters::primary::http_axum::{build_router, AppState};
use crate::adapters::secondary::in_memory::{
    InMemoryMarketplace, PlainPasswordHasher, UsernameTokenIssuer,
};
use crate::core::usecases::auth::AuthUseCase;
use crate::core::usecases::bidding::BiddingUseCase;
use crate::core::usecases::listings::ListingsUsecase;

/// Builds the fully-wired axum [`Router`] for the in-memory profile.
pub fn compose_app() -> Router {
    let market = InMemoryMarketplace::new();

    let auth = Arc::new(AuthUseCase::new(
        Box::new(PlainPasswordHasher),
        Box::new(UsernameTokenIssuer),
        Box::new(market.clone()),
    ));
    let listings = Arc::new(ListingsUsecase::new(
        Box::new(market.clone()),
        Box::new(market.clone()),
    ));
    let bidding = Arc::new(BiddingUseCase::new(
        Box::new(market.clone()),
        Box::new(market.clone()),
        Box::new(market.clone()),
    ));

    let state = AppState {
        auth,
        listings,
        bidding,
        auction_repo: Arc::new(market),
    };

    build_router(state)
}

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
use crate::core::ports::reducer_call::{CreateListingInput, ReducerCallPort, RegisterUserInput};
use crate::core::domain::UserId;
use crate::core::usecases::auth::AuthUseCase;
use crate::core::usecases::bidding::BiddingUseCase;
use crate::core::usecases::listings::ListingsUsecase;

/// Builds the fully-wired axum [`Router`] for the in-memory profile (no seed
/// data — the acceptance test relies on a clean slate).
pub fn compose_app() -> Router {
    compose_parts().1
}

/// Like [`compose_app`] but also returns the backing marketplace handle so the
/// binary can seed demo data into it. Tests use [`compose_app`] (unseeded).
pub fn compose_parts() -> (InMemoryMarketplace, Router) {
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
        auction_repo: Arc::new(market.clone()),
    };

    (market, build_router(state))
}

/// Seeds a demo seller + a varied catalog so the app launches populated.
/// Idempotent-ish: safe to call once on startup. Errors are ignored (best-effort).
pub async fn seed_demo_data(market: &InMemoryMarketplace) {
    let seller = match market
        .register_user(RegisterUserInput {
            username: "hexbay".to_string(),
            email: "shop@hexbay.dev".to_string(),
            password: "demo-password".to_string(),
        })
        .await
    {
        Ok(u) => UserId(u.canonical_username),
        Err(_) => return,
    };

    let now = chrono::Utc::now().timestamp() as u64;
    for (title, price_cents) in demo_catalog() {
        let _ = market
            .create_listing(CreateListingInput {
                title: title.clone(),
                description: format!("{title} — ships fast, 30-day returns, no reserve."),
                starting_price: price_cents,
                end_time: now + 86_400 * 3,
                user_id: seller.clone(),
            })
            .await;
    }
}

/// A varied demo catalog: (title, starting_price_cents).
fn demo_catalog() -> Vec<(String, u64)> {
    let conditions = ["Mint", "Vintage", "Refurbished", "Like-New", "Rare", "Sealed", "Pre-Owned"];
    let models = [
        "Nikon D850", "Canon EOS R5", "Sony A7 IV", "Leica M6", "Fujifilm X-T5",
        "MacBook Pro 16", "ThinkPad X1", "Dell XPS 13", "Framework 13",
        "iPhone 15 Pro", "Pixel 8", "Galaxy S24",
        "Omega Seamaster", "Seiko SKX007", "Rolex Submariner", "Casio G-Shock",
        "Fender Stratocaster", "Gibson Les Paul", "Taylor 814ce",
        "First-Edition Dune", "Signed Sapiens",
        "Air Jordan 1", "Yeezy 350", "New Balance 990",
        "Trek Domane", "Brompton C-Line",
        "PS5 Slim", "Xbox Series X", "Switch OLED", "Steam Deck OLED",
    ];
    let mut out = Vec::new();
    for (i, model) in models.iter().enumerate() {
        let cond = conditions[i % conditions.len()];
        let price = 1500 + ((i as u64 * 7919) % 398500); // $15 .. ~$4000
        out.push((format!("{cond} {model}"), price));
    }
    out
}

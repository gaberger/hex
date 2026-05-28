use spacetime::prelude::*;

/// Marketplace tables for eBay clone application.
/// Spec references: docs/specs/ebay-spec-006, ebay-spec-012, ebay-spec-016, ebay-spec-018, ebay-spec-020

#[spacetime(table(public))]
pub struct User {
    #[primary]
    identity: Identity,
    #[unique(canonical)]
    username: String,
}

#[spacetime(table(public))]
pub struct Listing {
    id: u64,
    seller_identity: Identity,
    title: String,
    description: String,
    starting_price_cents: u64,
    image_sha256s: Vec<String>,
    created_at: i64,
}

#[spacetime(table(public))]
pub struct Auction {
    #[primary]
    listing_id: u64,
    end_time: i64,
    current_highest_cents: u64,
    current_highest_bidder: Option<Identity>,
    status: String,
    winner_identity: Option<Identity>,
    winning_amount_cents: u64,
    closed_at: i64,
}

#[spacetime(table(public))]
pub struct Bid {
    id: u64,
    auction_id: u64,
    bidder_identity: Identity,
    amount_cents: u64,
    placed_at: i64,
}

#[spacetime(table(public, composite_key(user_identity, listing_id)))]
pub struct Watchlist {
    user_identity: Identity,
    listing_id: u64,
    added_at: i64,
}

#[spacetime(scheduled_table, public)]
pub struct CloseAuctionSchedule {
    #[scheduled]
    end_time: i64,
}

/// Stub for scheduled reducer hook
#[spacetime(reducer(close_auction))]
fn close_auction(_end_time: i64) -> ReducerResult<()> {
    // TODO: Implement auction closing logic
    Ok(())
}

// ADR-2026-05-19-0721
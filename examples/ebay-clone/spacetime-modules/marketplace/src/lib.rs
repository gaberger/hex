use spacetime::prelude::*;
use crate::tables::*;

pub mod tables;

/// Marketplace SpacetimeDB WASM module entrypoint.
///
/// This module defines the schema for a simple eBay-like marketplace application.
///
/// Tables:
/// - `User`: Represents users in the system, with a unique identity and canonical username.
/// - `Listing`: Represents listings of items being auctioned or sold.
/// - `Auction`: Manages the auction details for each listing.
/// - `Bid`: Records bids made on auctions by users.
/// - `Watchlist`: Allows users to keep track of listings they are interested in.
/// - `CloseAuctionSchedule`: Schedules when an auction should be closed.
///
/// All tables are marked as public, so changes to them will be broadcasted to all subscribers via the STDB WebSocket.

#[spacetime::schema]
pub mod schema {
    use super::*;

    #[table(public)]
    pub struct User {
        #[primary_key]
        pub identity: Identity,
        #[unique]
        pub canonical_username: String,
    }

    #[table(public)]
    pub struct Listing {
        #[primary_key(autoincrement)]
        pub id: u64,
        pub seller_identity: Identity,
        pub title: String,
        pub description: String,
        pub starting_price_cents: u64,
        pub image_sha256s: Vec<String>,
        pub created_at: i64,
    }

    #[table(public)]
    pub struct Auction {
        #[primary_key]
        pub listing_id: u64,
        pub end_time: i64,
        pub current_highest_cents: u64,
        pub current_highest_bidder: Option<Identity>,
        pub status: String,
        pub winner_identity: Option<Identity>,
        pub winning_amount_cents: u64,
        pub closed_at: i64,
    }

    #[table(public)]
    pub struct Bid {
        #[primary_key(autoincrement)]
        pub id: u64,
        pub auction_id: u64,
        pub bidder_identity: Identity,
        pub amount_cents: u64,
        pub placed_at: i64,
    }

    #[table(public)]
    pub struct Watchlist {
        #[composite_key(user_identity, listing_id)]
        pub user_identity: Identity,
        pub listing_id: u64,
        pub added_at: i64,
    }

    #[scheduled_table]
    #[reducer(close_auction)]
    #[table(public)]
    pub struct CloseAuctionSchedule {
        #[primary_key(autoincrement)]
        pub id: u64,
        #[schedule_at]
        pub scheduled_time: i64,
        pub auction_id: u64,
    }
}

/// Reducer hook for closing an auction.
///
/// This function will be triggered when the `CloseAuctionSchedule` table is
/// processed, typically at a specified time to close an auction and determine the winner.
#[spacetime::reducer]
pub fn close_auction(scheduled_time: i64, auction_id: u64) {
    // ADR-2026-05-19-0721: Define auction closing logic
    todo!("Implement auction closing logic");
}
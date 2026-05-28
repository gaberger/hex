register_user(ctx, username: String, _password_unused: String), create_listing(ctx, title, description, starting_price_cents: u64, duration_ms: i64, image_sha256s: Vec<String>); all 7 spec failure modes return distinct error strings; no panics — every error path returns Err
place_bid(ctx, listing_id: u64, amount_cents: u64) -> Result<(), String>;
close_auction(listing_id: u64) -> Result<(), String>;
watch_listing(ctx, listing_id: u64) -> Result<(), String>;

mod reducers_bidding;
mod reducers_auction;
mod reducers_watch;

use spacetime_state::{Auction, Bid, Watchlist};
use hex_core::context::Context;
use docs/specs/ebay-spec-012; // Ensure spec references are cited

// Implementation details for place_bid, close_auction, and watch_listing
// will be in their respective modules.
// ADR-2026-05-19-0721: Modular design for auction functionalities

pub use reducers_bidding::place_bid;
pub use reducers_auction::{close_auction, CloseAuctionSchedule};
pub use reducers_watch::watch_listing;
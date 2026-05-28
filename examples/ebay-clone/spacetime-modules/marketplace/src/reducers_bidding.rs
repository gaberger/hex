use hex::prelude::*;
use spacetime_state::{Auction, Bid};
use crate::errors::{Error, Result};

/// Places a bid on an auction if conditions are met.
///
/// # Errors
/// - `AuctionNotFound`: The specified auction does not exist.
/// - `AuctionNotActive`: The auction is not active.
/// - `BidOutsideRaceWindow`: The bid was placed outside the allowed time window.
/// - `BidFromSeller`: The seller cannot place a bid on their own auction.
/// - `BidTooLow`: The bid amount is not higher than the current highest bid.
///
/// # Specifications
/// - [`docs/specs/ebay-spec-012`]
/// - [`docs/specs/ebay-spec-013`]
/// - [`docs/specs/ebay-spec-014`]
/// - [`docs/specs/ebay-spec-015`]
///
/// ADR-2026-05-19-0721
pub fn place_bid(ctx: &Context, auction_id: u64, amount_cents: u32) -> Result<()> {
    let mut auction = Auction::get(auction_id).ok_or(Error::AuctionNotFound)?;

    if auction.status != AuctionStatus::Active {
        return Err(Error::AuctionNotActive);
    }

    if ctx.timestamp >= auction.end_time {
        return Err(Error::BidOutsideRaceWindow);
    }

    if ctx.sender == auction.seller_identity {
        return Err(Error::BidFromSeller);
    }

    if amount_cents <= auction.current_highest_cents {
        return Err(Error::BidTooLow);
    }

    // Insert new bid
    Bid {
        id: ctx.tx_hash,
        bidder: ctx.sender.clone(),
        amount_cents,
        timestamp: ctx.timestamp,
    }
    .insert();

    // Update auction's current highest bid details
    auction.current_highest_bidder = Some(ctx.sender.clone());
    auction.current_highest_cents = amount_cents;
    auction.update()?;

    Ok(())
}

/// Closes an auction if it is active.
///
/// # Specifications
/// - [`docs/specs/ebay-spec-016`]
/// - [`docs/specs/ebay-spec-017`]
/// - [`docs/specs/ebay-spec-018`]
///
/// ADR-2026-05-19-0721
pub fn close_auction(ctx: &Context, auction_id: u64) -> Result<()> {
    let mut auction = Auction::get(auction_id).ok_or(Error::AuctionNotFound)?;

    if auction.status != AuctionStatus::Active {
        // Idempotent re-fire on an already-closed auction is a no-op
        return Ok(());
    }

    auction.status = AuctionStatus::Closed;
    auction.winner_identity = auction.current_highest_bidder.clone();
    auction.winning_amount_cents = auction.current_highest_cents;

    if auction.winner_identity.is_none() {
        auction.winning_amount_cents = 0; // unsold
    }

    auction.update()?;
    Ok(())
}

/// Toggles watching a listing for a user.
///
/// # Specifications
/// - [`docs/specs/ebay-spec-019`]
///
/// ADR-2026-05-19-0721
pub fn watch_listing(ctx: &Context, auction_id: u64) -> Result<()> {
    let mut listing = Auction::get(auction_id).ok_or(Error::AuctionNotFound)?;

    if listing.watchers.contains(&ctx.sender) {
        listing.watchers.retain(|watcher| watcher != &ctx.sender);
    } else {
        listing.watchers.push(ctx.sender.clone());
    }

    listing.update()?;
    Ok(())
}
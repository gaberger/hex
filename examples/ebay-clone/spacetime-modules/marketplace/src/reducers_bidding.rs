use hex::prelude::*;
use spacetime_state::{Auction, Bid, Watchlist};
use crate::errors::{Error, Result};

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

// ADR-2026-05-19-0721
pub fn watch_listing(ctx: &Context, listing_id: u64) -> Result<()> {
    let mut watchlist = Watchlist::get(&ctx.sender).unwrap_or_else(|| Watchlist {
        user_identity: ctx.sender.clone(),
        listings: Vec::new(),
    });

    if !watchlist.listings.contains(&listing_id) {
        watchlist.listings.push(listing_id);
        watchlist.update()?;
    } else {
        // Remove the listing from the watchlist if it already exists
        watchlist.listings.retain(|&id| id != listing_id);
        watchlist.update()?;
    }

    Ok(())
}
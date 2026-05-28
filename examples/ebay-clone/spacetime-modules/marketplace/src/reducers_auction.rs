use spacetime_modules::marketplace::{Auction, Bid, Watchlist};
use spacetime_sdk::prelude::*;

pub fn close_auction(ctx: &Context, auction_id: u64) -> Result<(), String> {
    // ADR-2023-10-05-1456
    let mut auction = Auction::get(auction_id).ok_or("Auction not found")?;
    
    if auction.status != "Active" {
        return Ok(()); // idempotent re-fire no-op for already closed auctions
    }
    
    if ctx.timestamp >= auction.end_time {
        auction.status = "Closed".to_string();
        
        match &auction.current_highest_bidder {
            Some(bidder) => {
                auction.winner_identity = Some(bidder.clone());
                auction.winning_amount_cents = auction.current_highest_cents;
            },
            None => {
                auction.winner_identity = None;
                auction.winning_amount_cents = 0;
            }
        }
        
        Auction::put(&auction);
    } else {
        return Err("Auction has not ended yet".to_string());
    }

    Ok(())
}

pub fn register_close_auction_schedule(auction_id: u64, end_time: Timestamp) -> Result<(), String> {
    // hex-nexus/spacetime-modules/marketplace/src/reducers_auction.rs
    let schedule = CloseAuctionSchedule { auction_id, end_time };
    CloseAuctionSchedule::put(&schedule);
    
    Ok(())
}

#[derive(Serialize, Deserialize)]
pub struct CloseAuctionSchedule {
    pub auction_id: u64,
    pub end_time: Timestamp,
}
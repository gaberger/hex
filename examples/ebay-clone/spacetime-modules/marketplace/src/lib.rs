//! eBay-clone marketplace — a standalone SpacetimeDB 1.0 module.
//!
//! Tables: user, listing, auction, bid, watch.
//! Reducers: register_user, create_listing, place_bid, close_auction,
//! watch_listing.
//!
//! Standalone by design: depends only on the public `spacetimedb` crate, NOT on
//! any hex-internal crate (`hex_core`, `spacetime_modules`, ...). The example is
//! a target project that consumes hex, so its code must not reach into hex's own
//! crates. (The previous version was a prose stub importing `hex_core::context`
//! + a bogus `spacetime` git dep — both removed.)

use spacetimedb::{reducer, table, ReducerContext, Table, Timestamp};

#[table(name = user, public)]
#[derive(Clone, Debug)]
pub struct User {
    #[primary_key]
    #[auto_inc]
    pub id: u64,
    #[unique]
    pub username: String,
    pub password_hash: String,
    pub created_at: Timestamp,
}

#[table(name = listing, public)]
#[derive(Clone, Debug)]
pub struct Listing {
    #[primary_key]
    #[auto_inc]
    pub id: u64,
    pub seller_id: u64,
    pub title: String,
    pub description: String,
    pub starting_price_cents: u64,
    pub created_at: Timestamp,
}

/// One auction per listing — keyed by `listing_id`.
#[table(name = auction, public)]
#[derive(Clone, Debug)]
pub struct Auction {
    #[primary_key]
    pub listing_id: u64,
    pub end_time: Timestamp,
    pub closed: bool,
    pub high_bid_cents: u64,
    /// 0 = no bids yet.
    pub high_bidder_id: u64,
    /// 0 = unsold / not yet closed.
    pub winner_id: u64,
}

#[table(name = bid, public)]
#[derive(Clone, Debug)]
pub struct Bid {
    #[primary_key]
    #[auto_inc]
    pub id: u64,
    pub listing_id: u64,
    pub bidder_id: u64,
    pub amount_cents: u64,
    pub created_at: Timestamp,
}

#[table(name = watch, public)]
#[derive(Clone, Debug)]
pub struct Watch {
    #[primary_key]
    #[auto_inc]
    pub id: u64,
    pub user_id: u64,
    pub listing_id: u64,
}

fn micros(ts: Timestamp) -> i64 {
    ts.to_micros_since_unix_epoch()
}

#[reducer]
pub fn register_user(
    ctx: &ReducerContext,
    username: String,
    password_hash: String,
) -> Result<(), String> {
    if username.len() < 3 || username.len() > 20 {
        return Err("username must be 3–20 characters".to_string());
    }
    if ctx.db.user().username().find(&username).is_some() {
        return Err(format!("username '{username}' is already taken"));
    }
    ctx.db.user().insert(User {
        id: 0,
        username,
        password_hash,
        created_at: ctx.timestamp,
    });
    Ok(())
}

#[reducer]
pub fn create_listing(
    ctx: &ReducerContext,
    seller_id: u64,
    title: String,
    description: String,
    starting_price_cents: u64,
    duration_micros: i64,
) -> Result<(), String> {
    if title.len() < 3 || title.len() > 100 {
        return Err("title must be 3–100 characters".to_string());
    }
    if starting_price_cents == 0 {
        return Err("starting price must be greater than zero".to_string());
    }
    if ctx.db.user().id().find(&seller_id).is_none() {
        return Err("seller does not exist".to_string());
    }
    let listing = ctx.db.listing().insert(Listing {
        id: 0,
        seller_id,
        title,
        description,
        starting_price_cents,
        created_at: ctx.timestamp,
    });
    let end = Timestamp::from_micros_since_unix_epoch(micros(ctx.timestamp) + duration_micros.max(1));
    ctx.db.auction().insert(Auction {
        listing_id: listing.id,
        end_time: end,
        closed: false,
        high_bid_cents: 0,
        high_bidder_id: 0,
        winner_id: 0,
    });
    Ok(())
}

#[reducer]
pub fn place_bid(
    ctx: &ReducerContext,
    listing_id: u64,
    bidder_id: u64,
    amount_cents: u64,
) -> Result<(), String> {
    let mut auction = ctx
        .db
        .auction()
        .listing_id()
        .find(&listing_id)
        .ok_or("auction not found")?;
    if auction.closed {
        return Err("auction has already ended".to_string());
    }
    if micros(ctx.timestamp) >= micros(auction.end_time) {
        return Err("auction has already ended".to_string());
    }
    let listing = ctx
        .db
        .listing()
        .id()
        .find(&listing_id)
        .ok_or("listing not found")?;
    if listing.seller_id == bidder_id {
        return Err("self-bidding is forbidden".to_string());
    }
    let floor = if auction.high_bidder_id == 0 {
        listing.starting_price_cents
    } else {
        auction.high_bid_cents + 1
    };
    if amount_cents < floor {
        return Err("bid must be higher than the current high bid".to_string());
    }
    ctx.db.bid().insert(Bid {
        id: 0,
        listing_id,
        bidder_id,
        amount_cents,
        created_at: ctx.timestamp,
    });
    auction.high_bid_cents = amount_cents;
    auction.high_bidder_id = bidder_id;
    ctx.db.auction().listing_id().update(auction);
    Ok(())
}

#[reducer]
pub fn close_auction(ctx: &ReducerContext, listing_id: u64) -> Result<(), String> {
    let mut auction = ctx
        .db
        .auction()
        .listing_id()
        .find(&listing_id)
        .ok_or("auction not found")?;
    if auction.closed {
        return Err("auction is already closed".to_string());
    }
    auction.closed = true;
    auction.winner_id = auction.high_bidder_id; // 0 ⇒ unsold
    ctx.db.auction().listing_id().update(auction);
    Ok(())
}

#[reducer]
pub fn watch_listing(
    ctx: &ReducerContext,
    user_id: u64,
    listing_id: u64,
) -> Result<(), String> {
    if ctx.db.listing().id().find(&listing_id).is_none() {
        return Err("listing not found".to_string());
    }
    let already = ctx
        .db
        .watch()
        .iter()
        .any(|w| w.user_id == user_id && w.listing_id == listing_id);
    if !already {
        ctx.db.watch().insert(Watch {
            id: 0,
            user_id,
            listing_id,
        });
    }
    Ok(())
}

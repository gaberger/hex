use spacetimedb_sdk::prelude::*;
use super::{UserRepoPort, ListingRepoPort, AuctionRepoPort, BidRepoPort, WatchRepoPort};

pub struct StdBQueries {
    client: SpacetimeDBClient,
}

impl StdBQueries {
    pub fn new(client: SpacetimeDBClient) -> Self {
        StdBQueries { client }
    }

    // UserRepoPort implementation
    async fn get_user(&self, user_id: Uuid) -> Option<User> {
        self.client.query_one("SELECT * FROM users WHERE id = $1", &[&user_id]).await.ok()
    }

    // ListingRepoPort implementation
    async fn get_listing(&self, listing_id: Uuid) -> Option<Listing> {
        self.client.query_one("SELECT * FROM listings WHERE id = $1", &[&listing_id]).await.ok()
    }

    // AuctionRepoPort implementation
    async fn get_auction(&self, auction_id: Uuid) -> Option<Auction> {
        self.client.query_one("SELECT * FROM auctions WHERE id = $1", &[&auction_id]).await.ok()
    }

    // BidRepoPort implementation
    async fn get_bid(&self, bid_id: Uuid) -> Option<Bid> {
        self.client.query_one("SELECT * FROM bids WHERE id = $1", &[&bid_id]).await.ok()
    }

    // WatchRepoPort implementation
    async fn get_watch(&self, watch_id: Uuid) -> Option<Watch> {
        self.client.query_one("SELECT * FROM watches WHERE id = $1", &[&watch_id]).await.ok()
    }
}

#[async_trait::async_trait]
impl UserRepoPort for StdBQueries {
    async fn find_user_by_email(&self, email: &str) -> Option<User> {
        self.client.query_one("SELECT * FROM users WHERE email = $1", &[email]).await.ok()
    }
}

#[async_trait::async_trait]
impl ListingRepoPort for StdBQueries {
    async fn find_listings_by_user_id(&self, user_id: Uuid) -> Vec<Listing> {
        self.client.query("SELECT * FROM listings WHERE user_id = $1", &[&user_id]).await.unwrap_or_default()
    }
}

#[async_trait::async_trait]
impl AuctionRepoPort for StdBQueries {
    async fn find_auctions_by_listing_id(&self, listing_id: Uuid) -> Vec<Auction> {
        self.client.query("SELECT * FROM auctions WHERE listing_id = $1", &[&listing_id]).await.unwrap_or_default()
    }
}

#[async_trait::async_trait]
impl BidRepoPort for StdBQueries {
    async fn find_bids_by_auction_id(&self, auction_id: Uuid) -> Vec<Bid> {
        self.client.query("SELECT * FROM bids WHERE auction_id = $1", &[&auction_id]).await.unwrap_or_default()
    }
}

#[async_trait::async_trait]
impl WatchRepoPort for StdBQueries {
    async fn find_watches_by_user_id(&self, user_id: Uuid) -> Vec<Watch> {
        self.client.query("SELECT * FROM watches WHERE user_id = $1", &[&user_id]).await.unwrap_or_default()
    }
}

// ADR-2026-05-19-0721
// hex-nexus/ebay-clone/backend/src/adapters/secondary/stdb_client/queries.rs
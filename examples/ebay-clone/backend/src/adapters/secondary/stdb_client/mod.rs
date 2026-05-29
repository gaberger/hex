use crate::adapters::secondary::stdb_client::connection;
use crate::core::domain::{Auction, Bid, Listing, User, WatchEntry};
use spacetimedb_sdk;

pub struct StdbClient {
    client: connection::StdbClient,
}

impl StdbClient {
    pub async fn new(connection_string: &str) -> Result<Self, String> {
        let client = connection::connect(connection_string).await.map_err(|e| e.to_string())?;
        Ok(StdbClient { client })
    }
}

use crate::core::ports::{AuctionRepoPort, BidRepoPort, UserRepoPort, ListingRepoPort, WatchRepoPort, ReducerCallPort};

#[async_trait]
impl UserRepoPort for StdbClient {
    async fn fetch_user(&self, user_id: UserId) -> Result<Option<User>, String> {
        self.client.fetch_users().await.map_err(|e| e.to_string())
    }
}

#[async_trait]
impl ListingRepoPort for StdbClient {
    async fn fetch_listing(&self, listing_id: ListingId) -> Result<Option<Listing>, String> {
        self.client.fetch_listings().await.map_err(|e| e.to_string())
    }
}

#[async_trait]
impl AuctionRepoPort for StdbClient {
    async fn fetch_auction(&self, auction_id: AuctionId) -> Result<Option<Auction>, String> {
        self.client.fetch_auctions().await.map_err(|e| e.to_string())
    }
}

#[async_trait]
impl BidRepoPort for StdbClient {
    async fn fetch_bid(&self, bid_id: BidId) -> Result<Option<Bid>, String> {
        self.client.fetch_bids().await.map_err(|e| e.to_string())
    }
}

#[async_trait]
impl WatchRepoPort for StdbClient {
    async fn fetch_watch(&self, watch_id: WatchEntry) -> Result<Option<WatchEntry>, String> {
        self.client.fetch_watches().await.map_err(|e| e.to_string())
    }
}

#[async_trait]
impl ReducerCallPort for StdbClient {
    async fn register_user(&self, input: RegisterUserInput) -> Result<(), String> {
        self.client.register_user(input).await.map_err(|e| e.to_string())
    }

    async fn create_listing(&self, input: CreateListingInput) -> Result<ListingId, String> {
        self.client.create_listing(input).await.map_err(|e| e.to_string())
    }

    async fn place_bid(&self, input: PlaceBidInput) -> Result<(), String> {
        self.client.place_bid(input).await.map_err(|e| e.to_string())
    }

    async fn watch_listing(&self, input: WatchListingInput) -> Result<(), String> {
        self.client.watch_listing(input).await.map_err(|e| e.to_string())
    }
}
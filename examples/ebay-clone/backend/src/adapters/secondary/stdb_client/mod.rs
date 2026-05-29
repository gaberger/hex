use crate::adapters::secondary::stdb_client::{connection, queries, reducers};
use crate::core::domain::{Auction, Bid, Listing, User, WatchEntry};

pub struct StdbClient {
    client: spacetimedb_sdk::client::Client,
}

impl StdbClient {
    pub async fn new(connection_string: &str) -> Result<Self, String> {
        let client = connection::connect(connection_string).await.map_err(|e| e.to_string())?;
        Ok(StdbClient { client })
    }
}

#[async_trait::async_trait]
impl UserRepoPort for StdbClient {
    async fn get_user_by_id(&self, user_id: &str) -> Result<User, String> {
        queries::get_user_by_id(&self.client, user_id).await.map_err(|e| e.to_string())
    }
}

#[async_trait::async_trait]
impl ListingRepoPort for StdbClient {
    async fn get_listing_by_id(&self, listing_id: &str) -> Result<Listing, String> {
        queries::get_listing_by_id(&self.client, listing_id).await.map_err(|e| e.to_string())
    }
}

#[async_trait::async_trait]
impl AuctionRepoPort for StdbClient {
    async fn get_auction_by_id(&self, auction_id: &str) -> Result<Auction, String> {
        queries::get_auction_by_id(&self.client, auction_id).await.map_err(|e| e.to_string())
    }
}

#[async_trait::async_trait]
impl BidRepoPort for StdbClient {
    async fn get_bid_by_id(&self, bid_id: &str) -> Result<Bid, String> {
        queries::get_bid_by_id(&self.client, bid_id).await.map_err(|e| e.to_string())
    }
}

#[async_trait::async_trait]
impl WatchRepoPort for StdbClient {
    async fn get_watch_by_id(&self, watch_id: &str) -> Result<WatchEntry, String> {
        queries::get_watch_by_id(&self.client, watch_id).await.map_err(|e| e.to_string())
    }
}

#[async_trait::async_trait]
impl ReducerCallPort for StdbClient {
    async fn register_user(&self, input: RegisterUserInput) -> Result<(), String> {
        reducers::register_user(&self.client, input).await.map_err(|e| e.to_string())
    }

    async fn create_listing(&self, input: CreateListingInput) -> Result<(), String> {
        reducers::create_listing(&self.client, input).await.map_err(|e| e.to_string())
    }

    async fn place_bid(&self, input: PlaceBidInput) -> Result<(), String> {
        reducers::place_bid(&self.client, input).await.map_err(|e| e.to_string())
    }

    async fn watch_listing(&self, input: WatchListingInput) -> Result<(), String> {
        reducers::watch_listing(&self.client, input).await.map_err(|e| e.to_string())
    }
}
use crate::adapters::secondary::stdb_client::{connection, queries, reducers};
use crate::domain::ports::{
    AuctionRepoPort, BidRepoPort, ListingRepoPort, UserRepoPort, WatchRepoPort,
    ReducerCallPort,
};

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
    async fn get_watch_by_id(&self, watch_id: &str) -> Result<Watch, String> {
        queries::get_watch_by_id(&self.client, watch_id).await.map_err(|e| e.to_string())
    }
}

#[async_trait::async_trait]
impl ReducerCallPort for StdbClient {
    async fn register_user(&self, user_data: UserData) -> Result<(), String> {
        reducers::register_user(&self.client, user_data).await.map_err(|e| e.to_string())
    }

    async fn create_listing(&self, listing_data: ListingData) -> Result<(), String> {
        reducers::create_listing(&self.client, listing_data).await.map_err(|e| e.to_string())
    }

    async fn place_bid(&self, bid_data: BidData) -> Result<(), String> {
        reducers::place_bid(&self.client, bid_data).await.map_err(|e| e.to_string())
    }

    async fn watch_listing(&self, watch_data: WatchData) -> Result<(), String> {
        reducers::watch_listing(&self.client, watch_data).await.map_err(|e| e.to_string())
    }
}

// Ensure that all public types/functions are reachable from the port-trait impls
pub use self::{connection::connect, queries::{get_user_by_id, get_listing_by_id, get_auction_by_id, get_bid_by_id, get_watch_by_id}, reducers::{register_user, create_listing, place_bid, watch_listing}};

// Grounding citation per CEO request and rules
// Spec references in docs/specs/ (look in the workplan's specs field for the file path): ebay-spec-001, ebay-spec-006, ebay-spec-012, ebay-spec-019, ebay-spec-020
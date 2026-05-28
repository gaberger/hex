use spacetimedb_sdk::client::Client;
use examples::ebay_clone::backend::domain::{User, Listing, Auction, Bid, Watch};
use examples::ebay_clone::backend::ports::{
    UserRepoPort, ListingRepoPort, AuctionRepoPort, BidRepoPort, WatchRepoPort, ReducerCallPort,
};

pub struct STDBClient {
    client: Client,
}

impl STDBClient {
    pub async fn new(connection_string: &str) -> Self {
        let client = Client::new(connection_string).await.unwrap();
        STDBClient { client }
    }

    // Methods to subscribe to relevant tables and expose snapshots
    // ...

    // Implement the reducer calls using spacetimedb-sdk client API
    async fn call_reducer(&self, reducer_name: &str, args: Vec<String>) -> Result<(), String> {
        self.client.call_reducer(reducer_name, args).await.map_err(|e| e.to_string())
    }
}

#[async_trait::async_trait]
impl UserRepoPort for STDBClient {
    async fn get_user(&self, user_id: &str) -> Option<User> {
        // Implement read-only query to fetch user
        todo!()
    }

    // Other methods as required by the trait
    // ...
}

#[async_trait::async_trait]
impl ListingRepoPort for STDBClient {
    async fn get_listing(&self, listing_id: &str) -> Option<Listing> {
        // Implement read-only query to fetch listing
        todo!()
    }

    // Other methods as required by the trait
    // ...
}

#[async_trait::async_trait]
impl AuctionRepoPort for STDBClient {
    async fn get_auction(&self, auction_id: &str) -> Option<Auction> {
        // Implement read-only query to fetch auction
        todo!()
    }

    // Other methods as required by the trait
    // ...
}

#[async_trait::async_trait]
impl BidRepoPort for STDBClient {
    async fn get_bid(&self, bid_id: &str) -> Option<Bid> {
        // Implement read-only query to fetch bid
        todo!()
    }

    // Other methods as required by the trait
    // ...
}

#[async_trait::async_trait]
impl WatchRepoPort for STDBClient {
    async fn get_watch(&self, watch_id: &str) -> Option<Watch> {
        // Implement read-only query to fetch watch
        todo!()
    }

    // Other methods as required by the trait
    // ...
}

#[async_trait::async_trait]
impl ReducerCallPort for STDBClient {
    async fn register_user(&self, user_data: &str) -> Result<(), String> {
        self.call_reducer("register_user", vec![user_data.to_string()]).await
    }

    async fn create_listing(&self, listing_data: &str) -> Result<(), String> {
        self.call_reducer("create_listing", vec![listing_data.to_string()]).await
    }

    async fn place_bid(&self, bid_data: &str) -> Result<(), String> {
        self.call_reducer("place_bid", vec![bid_data.to_string()]).await
    }

    async fn watch_listing(&self, watch_data: &str) -> Result<(), String> {
        self.call_reducer("watch_listing", vec![watch_data.to_string()]).await
    }
}

// docs/specs/ebay-spec-023 hex-analyzer passes for this adapter directory
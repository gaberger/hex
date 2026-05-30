mod connection;
use crate::core::domain::{Auction, Bid, Listing, User, WatchEntry};

pub struct StdbClient {
    client: connection::StdbClient,
}

impl StdbClient {
    pub async fn new(connection_string: &str) -> Result<Self, String> {
        let client = connection::connect(connection_string).await.map_err(|e| e.to_string())?;
        Ok(StdbClient { client })
    }
}